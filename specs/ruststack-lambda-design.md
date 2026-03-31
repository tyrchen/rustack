# Rustack Lambda: Invoke-Focused Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [rustack-sqs-design.md](./rustack-sqs-design.md)
**Scope:** Add Lambda support to Rustack with Docker-based function execution, covering function CRUD, synchronous/asynchronous invocation, versions, aliases, and function URLs. No event source mappings, no layers management, no Lambda@Edge.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: restJson1](#5-protocol-design-restjson1)
6. [Smithy Code Generation Strategy](#6-smithy-code-generation-strategy)
7. [Crate Structure](#7-crate-structure)
8. [HTTP Layer Design](#8-http-layer-design)
9. [Execution Engine Design](#9-execution-engine-design)
10. [Storage Engine Design](#10-storage-engine-design)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [Server Integration](#13-server-integration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding Lambda support to Rustack as a native Rust control plane with Docker-based function execution. Key design decisions:

- **Invoke-focused scope** -- the primary goal is to let developers invoke Lambda functions locally via Docker containers. Function CRUD (create, update, delete, list) provides the management API; `Invoke` with `RequestResponse`, `Event`, and `DryRun` modes provides the execution API.
- **Docker-based execution engine** -- function code runs inside Docker containers using AWS Lambda base images (`public.ecr.aws/lambda/*`) which include the Lambda Runtime Interface Emulator (RIE). Rustack manages container lifecycle: pull images, start containers, forward invocation payloads, collect responses, and optionally reuse warm containers.
- **restJson1 protocol** -- Lambda uses `restJson1`, which is fundamentally different from the `awsJson1.0`/`awsJson1.1` protocols used by DynamoDB, SQS, and SSM. Operations are dispatched by HTTP method + URL path (e.g., `POST /2015-03-31/functions/{FunctionName}/invocations`) rather than `X-Amz-Target` headers. This requires a new routing approach in the HTTP layer.
- **Gateway routing by URL prefix** -- all Lambda API requests use URL paths starting with `/2015-03-31/functions` or `/2021-10-31/functions`. The gateway routes requests matching these prefixes to the Lambda service, before falling through to S3.
- **Smithy codegen adaptation** -- the existing codegen must be extended to handle `restJson1` protocol traits (`@http`, `@httpLabel`, `@httpQuery`, `@httpPayload`, `@httpHeader`) in addition to the existing `awsJson` support.
- **Phased delivery** -- 4 phases from MVP (CreateFunction + Invoke with zip deployment) to full feature set including versions, aliases, function URLs, and async invocation with internal queuing.

---

## 2. Motivation

### 2.1 Why Lambda?

Lambda is the centerpiece of AWS serverless architecture. Every serious AWS application uses Lambda either directly or indirectly through event-driven patterns. Developers need local Lambda for:

- **SAM CLI / CDK local testing** -- `sam local invoke` and `sam local start-lambda` need a Lambda-compatible API endpoint to test functions without deploying to AWS
- **Serverless Framework** -- `serverless invoke local` and integration testing workflows depend on local Lambda execution
- **Event-driven architecture** -- testing Lambda functions triggered by S3, SQS, DynamoDB Streams, or API Gateway events locally
- **CI/CD pipelines** -- fast Lambda invocation in GitHub Actions without AWS credentials or network access
- **Cost savings** -- avoid deploying to AWS for every code change during development
- **Offline development** -- work without internet connectivity

### 2.2 Why Not Wrap Existing Tools?

| Tool | Issue |
|------|-------|
| **SAM CLI local** | Only supports `sam local invoke` (single invocation), not a persistent Lambda API endpoint. Cannot be used as a drop-in replacement for the Lambda API. |
| **serverless-offline** | Node.js-specific, tightly coupled to the Serverless Framework. Not a general-purpose Lambda API emulator. |
| **docker-lambda** (lambci) | Archived/unmaintained since 2023. Was a Docker image collection, not an API server. |
| **LocalStack Lambda** | Python-based, requires the full LocalStack runtime (~1GB image). Not embeddable. |

### 2.3 What Rustack Lambda Provides

A lightweight, persistent Lambda API endpoint that:

- Accepts standard AWS SDK calls (`CreateFunction`, `Invoke`, etc.)
- Runs function code in Docker containers using official AWS Lambda base images
- Lives in the same ~15MB Docker image as S3, DynamoDB, SQS, and SSM
- Starts in milliseconds (control plane only; container startup is on first invoke)
- Works with SAM CLI, CDK, Serverless Framework, Terraform, and any AWS SDK

### 2.4 Tool Coverage

With the target operations implemented, the following tools work:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws lambda`) | All CRUD + Invoke | Phase 0 |
| SAM CLI (`sam local start-lambda`) | CreateFunction, Invoke, GetFunction | Phase 0 |
| Serverless Framework | CreateFunction, Invoke, UpdateFunctionCode | Phase 0 |
| AWS CDK (local testing) | CreateFunction, Invoke, GetFunction, UpdateFunctionConfiguration | Phase 0 |
| Terraform | CreateFunction, GetFunction, DeleteFunction, AddPermission | Phase 1 |
| boto3 / aws-sdk-rust | All operations | All phases |
| Function URLs (`curl`) | CreateFunctionUrlConfig, direct HTTP invoke | Phase 3 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Docker-based function execution** -- invoke Lambda functions locally using Docker containers with AWS Lambda base images and the Runtime Interface Emulator (RIE)
2. **Function CRUD** -- CreateFunction, GetFunction, UpdateFunctionCode, UpdateFunctionConfiguration, DeleteFunction, ListFunctions, GetFunctionConfiguration
3. **Synchronous invocation** -- `RequestResponse` mode with payload forwarding to container and response collection
4. **Asynchronous invocation** -- `Event` mode with internal queue and background execution
5. **DryRun invocation** -- validate invocation parameters without executing
6. **Zip and container image deployment** -- support both `.zip` file upload (base64 in `ZipFile` field) and container image URI (`ImageUri` field)
7. **Versions and aliases** -- PublishVersion, ListVersionsByFunction, CreateAlias, GetAlias, UpdateAlias, DeleteAlias, ListAliases
8. **Function URLs** -- CreateFunctionUrlConfig, GetFunctionUrlConfig, UpdateFunctionUrlConfig, DeleteFunctionUrlConfig for direct HTTP invocation
9. **Permissions** -- AddPermission, RemovePermission, GetPolicy (store but do not enforce)
10. **Tags** -- TagResource, UntagResource, ListTags
11. **Warm container reuse** -- keep containers alive between invocations to simulate warm starts
12. **Environment variables** -- pass function environment variables to containers
13. **restJson1 protocol** -- URL-based routing with proper HTTP method/path/status code binding
14. **Smithy-generated types** -- all Lambda API types generated from official AWS Smithy model
15. **Same Docker image** -- single binary serves S3, DynamoDB, SQS, SSM, and Lambda on port 4566

### 3.2 Non-Goals

1. **Event source mappings** -- no SQS, DynamoDB Streams, Kinesis, or S3 event triggers. Design should allow adding these later but they are out of scope.
2. **Layers management** -- accept `Layers` parameter in CreateFunction but do not download or mount layer content. Store metadata only.
3. **Lambda@Edge** -- no CloudFront integration
4. **Provisioned concurrency** -- accept configuration but do not pre-warm containers
5. **Code signing** -- accept CodeSigningConfigArn but do not validate signatures
6. **VPC configuration** -- accept VpcConfig but do not create network namespaces
7. **X-Ray tracing** -- accept TracingConfig but do not emit traces
8. **CloudWatch Logs integration** -- container stdout/stderr is captured and returned in `LogResult` header but not persisted to a CloudWatch-compatible API
9. **IAM policy enforcement** -- accept permissions but do not evaluate resource-based policies
10. **Concurrency limits** -- accept ReservedConcurrentExecutions but do not enforce throttling
11. **SnapStart** -- accept configuration, do not implement JVM snapshot/restore
12. **InvokeWithResponseStream** -- streaming response mode is out of scope for MVP
13. **S3-based deployment packages** -- accept S3Bucket/S3Key parameters but do not fetch from S3. Only `ZipFile` (inline) and `ImageUri` (Docker) are functional.
14. **Data persistence across restarts** -- in-memory only, matching other services
15. **Durable executions** -- new Lambda feature, out of scope

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / SAM CLI
                         |
                         | HTTP :4566
                         v
              +---------------------+
              |   Gateway Router    |  Routes by URL prefix or X-Amz-Target
              |   (ServiceRouter)   |
              +--------+------------+
                       |
         +------+------+------+------+------+
         v      v      v      v      v      v
     +------+ +-----+ +-----+ +-----+ +--------+
     | S3   | | DDB | | SQS | | SSM | | Lambda |
     | Rest | | J10 | | J10 | | J11 | | RestJ1 |
     | Xml  | |     | | +Qry| |     | |        |
     +------+ +-----+ +-----+ +-----+ +--------+
         |      |      |      |        |
     +------+ +-----+ +-----+ +-----+ +--------+
     | S3   | | DDB | | SQS | | SSM | | Lambda |
     | Core | | Core| | Core| | Core| | Core   |
     +------+ +-----+ +-----+ +-----+ +---+----+
         |      |      |      |            |
         +------+------+------+        +---+----+
                |                      | Docker  |
         +------+------+              | Engine  |
         | rustack-  |              +---+-----+
         | core + auth |                  |
         +-------------+              containers
```

### 4.2 Gateway Routing

Lambda uses `restJson1`, which routes by HTTP method and URL path. This is different from all other Rustack services that use `X-Amz-Target` or content-type-based dispatch.

| Signal | S3 | DynamoDB | SQS | SSM | Lambda |
|--------|----|---------:|-----|-----|--------|
| HTTP Method | Any | POST | POST | POST | GET/POST/PUT/DELETE |
| URL Path | `/{bucket}/{key}` | `/` | `/` | `/` | `/2015-03-31/functions/*` or `/2021-10-31/functions/*` |
| Content-Type | varies | `x-amz-json-1.0` | `x-amz-json-1.0` | `x-amz-json-1.1` | `application/json` |
| X-Amz-Target | absent | `DynamoDB_*` | `AmazonSQS.*` | `AmazonSSM.*` | absent |
| Dispatch | URL path | Header | Header | Header | URL path + method |

**Routing logic** (evaluated in order):
1. If URL path starts with `/2015-03-31/functions` or `/2021-10-31/functions` -- route to Lambda
2. If `X-Amz-Target` starts with `DynamoDB_` -- route to DynamoDB
3. If `X-Amz-Target` starts with `AmazonSQS` -- route to SQS (JSON protocol)
4. If `X-Amz-Target` starts with `AmazonSSM.` -- route to SSM
5. If `Content-Type` is `application/x-www-form-urlencoded` and POST to `/` -- route to SQS (Query)
6. Default: route to S3 (catch-all)

Lambda must be registered before S3 in the gateway because S3 is the catch-all. The URL prefix `/2015-03-31/functions` is unambiguous and cannot conflict with S3 bucket names (they cannot contain `/` as the first character of a path segment at the root level in this format).

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-lambda-model      <-- NEW (auto-generated)
+-- rustack-lambda-core       <-- NEW
+-- rustack-lambda-http       <-- NEW

rustack-lambda-http
+-- rustack-lambda-model
+-- rustack-auth

rustack-lambda-core
+-- rustack-core
+-- rustack-lambda-model
+-- bollard (Docker API client)
+-- tokio (channels, tasks, timers)
+-- dashmap

rustack-lambda-model (auto-generated, standalone)
```

---

## 5. Protocol Design: restJson1

### 5.1 Protocol Characteristics

Lambda uses the `restJson1` Smithy protocol, which fundamentally differs from the `awsJson1.0`/`awsJson1.1` protocols used by DynamoDB, SQS, and SSM:

| Aspect | awsJson (DDB/SQS/SSM) | restJson1 (Lambda) |
|--------|----------------------|---------------------|
| HTTP Method | POST only | GET, POST, PUT, DELETE |
| URL Path | Always `/` | Operation-specific (e.g., `/2015-03-31/functions/{name}`) |
| Operation dispatch | `X-Amz-Target` header | HTTP method + URL path matching |
| Request params | All in JSON body | Split across path, query, headers, and body |
| Response status | Always 200 (success) | Operation-specific (200, 201, 202, 204) |
| Error type header | `x-amzn-query-error` (SQS) | `X-Amzn-Errortype` |
| Content-Type | `application/x-amz-json-1.0` or `1.1` | `application/json` |

### 5.2 Request Anatomy

A typical Lambda request binds parameters across multiple HTTP locations:

```http
POST /2015-03-31/functions/my-function/invocations?Qualifier=v1 HTTP/1.1
X-Amz-Invocation-Type: RequestResponse
X-Amz-Log-Type: Tail
Content-Type: application/json

{"key": "value"}
```

Breakdown:
- **URL path label**: `FunctionName` = `my-function` (via `@httpLabel`)
- **Query parameter**: `Qualifier` = `v1` (via `@httpQuery`)
- **Header**: `InvocationType` = `RequestResponse` (via `@httpHeader("X-Amz-Invocation-Type")`)
- **Header**: `LogType` = `Tail` (via `@httpHeader("X-Amz-Log-Type")`)
- **Body payload**: `{"key": "value"}` (via `@httpPayload`)

### 5.3 Response Anatomy

```http
HTTP/1.1 200 OK
Content-Type: application/json
X-Amz-Function-Error: Unhandled
X-Amz-Log-Result: <base64-encoded-last-4KB-of-logs>
X-Amz-Executed-Version: 1

{"errorMessage": "something went wrong", "errorType": "Error"}
```

Response binding:
- **Status code**: 200 for `RequestResponse`, 202 for `Event`, 204 for `DryRun`
- **Headers**: `FunctionError`, `LogResult`, `ExecutedVersion` (via `@httpHeader`)
- **Body payload**: function response or error (via `@httpPayload`)

### 5.4 Error Response Format

Lambda errors use the `X-Amzn-Errortype` header (note: different from the `x-amzn-query-error` header used by SQS):

```http
HTTP/1.1 404 Not Found
Content-Type: application/json
X-Amzn-Errortype: ResourceNotFoundException

{"Type": "User", "Message": "Function not found: arn:aws:lambda:us-east-1:000000000000:function:nonexistent"}
```

Error response body fields:
- `Type`: `"User"` for client errors (4xx), `"Service"` for server errors (5xx)
- `Message`: human-readable error description
- Note: Lambda uses `Message` (capital M), not `message` (lowercase) like DynamoDB/SSM

### 5.5 Lambda API Route Table

All Lambda operations with their HTTP bindings:

| Operation | Method | Path | Success Status |
|-----------|--------|------|---------------|
| **CreateFunction** | POST | `/2015-03-31/functions` | 201 |
| **GetFunction** | GET | `/2015-03-31/functions/{FunctionName}` | 200 |
| **GetFunctionConfiguration** | GET | `/2015-03-31/functions/{FunctionName}/configuration` | 200 |
| **UpdateFunctionCode** | PUT | `/2015-03-31/functions/{FunctionName}/code` | 200 |
| **UpdateFunctionConfiguration** | PUT | `/2015-03-31/functions/{FunctionName}/configuration` | 200 |
| **DeleteFunction** | DELETE | `/2015-03-31/functions/{FunctionName}` | 204 |
| **ListFunctions** | GET | `/2015-03-31/functions` | 200 |
| **Invoke** | POST | `/2015-03-31/functions/{FunctionName}/invocations` | 200/202/204 |
| **PublishVersion** | POST | `/2015-03-31/functions/{FunctionName}/versions` | 201 |
| **ListVersionsByFunction** | GET | `/2015-03-31/functions/{FunctionName}/versions` | 200 |
| **CreateAlias** | POST | `/2015-03-31/functions/{FunctionName}/aliases` | 201 |
| **GetAlias** | GET | `/2015-03-31/functions/{FunctionName}/aliases/{Name}` | 200 |
| **UpdateAlias** | PUT | `/2015-03-31/functions/{FunctionName}/aliases/{Name}` | 200 |
| **DeleteAlias** | DELETE | `/2015-03-31/functions/{FunctionName}/aliases/{Name}` | 204 |
| **ListAliases** | GET | `/2015-03-31/functions/{FunctionName}/aliases` | 200 |
| **AddPermission** | POST | `/2015-03-31/functions/{FunctionName}/policy` | 201 |
| **RemovePermission** | DELETE | `/2015-03-31/functions/{FunctionName}/policy/{StatementId}` | 204 |
| **GetPolicy** | GET | `/2015-03-31/functions/{FunctionName}/policy` | 200 |
| **TagResource** | POST | `/2015-03-31/tags/{Resource}` | 204 |
| **UntagResource** | DELETE | `/2015-03-31/tags/{Resource}` | 204 |
| **ListTags** | GET | `/2015-03-31/tags/{Resource}` | 200 |
| **GetAccountSettings** | GET | `/2015-03-31/account-settings` | 200 |
| **CreateFunctionUrlConfig** | POST | `/2021-10-31/functions/{FunctionName}/url` | 201 |
| **GetFunctionUrlConfig** | GET | `/2021-10-31/functions/{FunctionName}/url` | 200 |
| **UpdateFunctionUrlConfig** | PUT | `/2021-10-31/functions/{FunctionName}/url` | 200 |
| **DeleteFunctionUrlConfig** | DELETE | `/2021-10-31/functions/{FunctionName}/url` | 204 |
| **ListFunctionUrlConfigs** | GET | `/2021-10-31/functions/{FunctionName}/urls` | 200 |

Total: **27 operations** across management, invocation, versioning, aliases, permissions, tags, and function URLs.

### 5.6 Qualifier Resolution

Many operations accept a `Qualifier` query parameter or embed a qualifier in the function name:

- `my-function` -- resolves to `$LATEST`
- `my-function:v1` -- resolves to alias `v1`
- `my-function:3` -- resolves to version `3`
- `my-function:$LATEST` -- resolves to unpublished version
- `arn:aws:lambda:us-east-1:000000000000:function:my-function:v1` -- ARN with qualifier

The resolver extracts the function name and optional qualifier, then looks up the target version.

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Extend Codegen for restJson1

The existing codegen supports `awsJson1.0` (DynamoDB, SQS) and `awsJson1.1` (SSM). Lambda's `restJson1` protocol requires a new code generation path that emits HTTP binding metadata alongside the model types.

The key difference: for `awsJson` services, the codegen generates flat input/output structs with serde derives. For `restJson1`, the codegen must additionally generate:
- Route metadata (HTTP method, URL path template, success status code)
- Field binding annotations (which fields go in path, query, headers, body)
- An operation router that matches incoming HTTP requests to operations

### 6.2 Lambda Service Config

```rust
const LAMBDA_OPERATIONS: &[&str] = &[
    "CreateFunction",
    "GetFunction",
    "GetFunctionConfiguration",
    "UpdateFunctionCode",
    "UpdateFunctionConfiguration",
    "DeleteFunction",
    "ListFunctions",
    "Invoke",
    "PublishVersion",
    "ListVersionsByFunction",
    "CreateAlias",
    "GetAlias",
    "UpdateAlias",
    "DeleteAlias",
    "ListAliases",
    "AddPermission",
    "RemovePermission",
    "GetPolicy",
    "TagResource",
    "UntagResource",
    "ListTags",
    "GetAccountSettings",
    "CreateFunctionUrlConfig",
    "GetFunctionUrlConfig",
    "UpdateFunctionUrlConfig",
    "DeleteFunctionUrlConfig",
    "ListFunctionUrlConfigs",
];

pub struct LambdaServiceConfig;

impl ServiceConfig for LambdaServiceConfig {
    fn namespace(&self) -> &str { "com.amazonaws.lambda#" }
    fn service_name(&self) -> &str { "Lambda" }
    fn target_operations(&self) -> &[&str] { &LAMBDA_OPERATIONS }
    fn protocol(&self) -> Protocol { Protocol::RestJson1 }
}
```

### 6.3 Key Differences from awsJson Codegen

| Aspect | awsJson (DDB/SQS/SSM) | restJson1 (Lambda) |
|--------|----------------------|---------------------|
| Input struct serde | All fields in JSON body | Only `@httpPayload` or non-bound fields in body |
| Path parameters | N/A | `@httpLabel` fields extracted from URL path |
| Query parameters | N/A | `@httpQuery` fields extracted from query string |
| Header parameters | N/A | `@httpHeader` fields extracted/set via headers |
| Operation enum | Dispatched by string name | Dispatched by method + path pattern |
| Error type header | `__type` in body | `X-Amzn-Errortype` header |

### 6.4 Generated Route Metadata

For each operation, the codegen emits a route descriptor:

```rust
/// Auto-generated route table for Lambda operations.
pub struct LambdaRoute {
    pub method: http::Method,
    pub path_pattern: &'static str,
    pub operation: LambdaOperation,
    pub success_status: u16,
}

pub const LAMBDA_ROUTES: &[LambdaRoute] = &[
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions",
        operation: LambdaOperation::CreateFunction,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}",
        operation: LambdaOperation::GetFunction,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions/{FunctionName}/invocations",
        operation: LambdaOperation::Invoke,
        success_status: 200,
    },
    // ... all 27 operations
];
```

### 6.5 Generated Input/Output Field Bindings

```rust
/// Field binding location for restJson1 protocol.
#[derive(Debug, Clone, Copy)]
pub enum FieldBinding {
    /// Bound to a URL path segment (e.g., /functions/{FunctionName}).
    PathLabel,
    /// Bound to a query string parameter.
    QueryParam(&'static str),
    /// Bound to an HTTP header.
    Header(&'static str),
    /// The entire struct or field is the JSON body payload.
    Payload,
    /// Part of the default JSON body (non-annotated fields).
    Body,
}
```

### 6.6 Smithy Model Acquisition

The Lambda Smithy model is available from:
- **Repository:** `https://github.com/aws/aws-models`
- **Path:** `lambda/smithy/model.json`
- Download and place at `codegen/smithy-model/lambda.json`

### 6.7 Generated Types Estimate

From the 27 operations, the codegen produces roughly:
- 27 input structs (e.g., `CreateFunctionInput`, `InvokeInput`)
- 27 output structs (e.g., `CreateFunctionOutput`, `InvokeOutput`)
- ~40 shared types (`FunctionConfiguration`, `FunctionCode`, `Environment`, `VpcConfig`, `Runtime`, `Architecture`, `PackageType`, `State`, `InvocationType`, `LogType`, `AliasConfiguration`, `FunctionUrlConfig`, etc.)
- 1 operation enum (`LambdaOperation` with 27 variants)
- 27 route descriptors
- ~25 error types

Total: roughly 3,000-4,000 lines of generated code.

### 6.8 Makefile Integration

```makefile
codegen-lambda:
	@cd codegen && cargo run -- --service lambda
	@cargo +nightly fmt -p rustack-lambda-model

codegen: codegen-s3 codegen-dynamodb codegen-sqs codegen-ssm codegen-lambda
```

---

## 7. Crate Structure

### 7.1 New Crates

#### `rustack-lambda-model` (auto-generated)

```
crates/rustack-lambda-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # Runtime, Architecture, PackageType, State, etc.
    +-- operations.rs             # LambdaOperation enum + route table
    +-- error.rs                  # LambdaError + error codes
    +-- input/
    |   +-- mod.rs
    |   +-- function.rs           # CreateFunctionInput, GetFunctionInput, etc.
    |   +-- invoke.rs             # InvokeInput
    |   +-- version.rs            # PublishVersionInput, ListVersionsByFunctionInput
    |   +-- alias.rs              # CreateAliasInput, GetAliasInput, etc.
    |   +-- permission.rs         # AddPermissionInput, RemovePermissionInput, GetPolicyInput
    |   +-- tag.rs                # TagResourceInput, UntagResourceInput, ListTagsInput
    |   +-- url.rs                # CreateFunctionUrlConfigInput, etc.
    +-- output/
        +-- mod.rs
        +-- function.rs           # FunctionConfiguration (shared by many responses)
        +-- invoke.rs             # InvokeOutput (with payload + headers)
        +-- version.rs
        +-- alias.rs
        +-- permission.rs
        +-- tag.rs
        +-- url.rs
```

**Dependencies**: `serde`, `serde_json`, `bytes`, `http`

#### `rustack-lambda-http`

```
crates/rustack-lambda-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # URL path + method pattern matching -> LambdaOperation
    +-- dispatch.rs               # LambdaHandler trait + dispatch logic
    +-- service.rs                # Hyper Service impl for Lambda
    +-- request.rs                # restJson1 request deserialization (path, query, headers, body)
    +-- response.rs               # restJson1 response serialization (status, headers, body)
    +-- error.rs                  # Error response with X-Amzn-Errortype header
    +-- path.rs                   # URL path parameter extraction
    +-- query.rs                  # Query string parameter extraction
    +-- body.rs                   # Response body type
```

**Dependencies**: `rustack-lambda-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`, `http`

#### `rustack-lambda-core`

```
crates/rustack-lambda-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # LambdaConfig
    +-- provider.rs               # RustackLambda (main provider, all operation handlers)
    +-- error.rs                  # LambdaServiceError
    +-- function/
    |   +-- mod.rs
    |   +-- state.rs              # FunctionRecord, VersionRecord, AliasRecord
    |   +-- storage.rs            # FunctionStore (DashMap-based function registry)
    |   +-- code.rs               # Code storage: zip extraction, temp dir management
    |   +-- resolver.rs           # Function name/ARN/qualifier resolution
    |   +-- validation.rs         # Function name, handler, runtime validation
    +-- execution/
    |   +-- mod.rs
    |   +-- engine.rs             # ExecutionEngine trait + DockerExecutionEngine
    |   +-- docker.rs             # Docker container lifecycle via bollard
    |   +-- container.rs          # ContainerPool: warm container reuse
    |   +-- runtime.rs            # Runtime -> Docker image mapping
    |   +-- invoke.rs             # Invoke orchestration: route to container, collect response
    +-- ops/
        +-- mod.rs
        +-- create.rs             # CreateFunction
        +-- get.rs                # GetFunction, GetFunctionConfiguration
        +-- update.rs             # UpdateFunctionCode, UpdateFunctionConfiguration
        +-- delete.rs             # DeleteFunction
        +-- list.rs               # ListFunctions
        +-- invoke.rs             # Invoke (RequestResponse, Event, DryRun)
        +-- version.rs            # PublishVersion, ListVersionsByFunction
        +-- alias.rs              # CreateAlias, GetAlias, UpdateAlias, DeleteAlias, ListAliases
        +-- permission.rs         # AddPermission, RemovePermission, GetPolicy
        +-- tag.rs                # TagResource, UntagResource, ListTags
        +-- url.rs                # Function URL CRUD
        +-- account.rs            # GetAccountSettings
```

**Dependencies**: `rustack-core`, `rustack-lambda-model`, `bollard`, `tokio` (rt, sync, time, process, fs), `dashmap`, `uuid`, `sha2`, `base64`, `tracing`, `tempfile`

### 7.2 Workspace Changes

```toml
[workspace.dependencies]
# ... existing deps ...
rustack-lambda-model = { path = "crates/rustack-lambda-model" }
rustack-lambda-http = { path = "crates/rustack-lambda-http" }
rustack-lambda-core = { path = "crates/rustack-lambda-core" }
bollard = "~0.18"
tempfile = "~3.15"
```

---

## 8. HTTP Layer Design

### 8.1 restJson1 Router

Unlike the `awsJson` services that dispatch on a single `X-Amz-Target` header, Lambda dispatches on the combination of HTTP method and URL path. The router uses pattern matching against the route table:

```rust
/// Lambda operation router.
///
/// Matches incoming HTTP requests against the Lambda route table
/// using method + path pattern matching.
pub struct LambdaRouter;

impl LambdaRouter {
    /// Resolve an HTTP request to a Lambda operation.
    ///
    /// Returns the matched operation and extracted path parameters.
    pub fn resolve(
        method: &http::Method,
        path: &str,
    ) -> Result<(LambdaOperation, PathParams), LambdaError> {
        // Try each route in order (most specific first).
        // Routes are ordered by path specificity:
        //   /2015-03-31/functions/{name}/invocations  (POST)
        //   /2015-03-31/functions/{name}/aliases/{alias}  (GET/PUT/DELETE)
        //   /2015-03-31/functions/{name}/aliases  (GET/POST)
        //   /2015-03-31/functions/{name}/versions  (GET/POST)
        //   /2015-03-31/functions/{name}/policy/{sid}  (DELETE)
        //   /2015-03-31/functions/{name}/policy  (GET/POST)
        //   /2015-03-31/functions/{name}/code  (PUT)
        //   /2015-03-31/functions/{name}/configuration  (GET/PUT)
        //   /2015-03-31/functions/{name}  (GET/DELETE)
        //   /2015-03-31/functions  (GET/POST)
        //   /2021-10-31/functions/{name}/url  (GET/POST/PUT/DELETE)
        //   /2015-03-31/tags/{arn}  (GET/POST/DELETE)
        //   /2015-03-31/account-settings  (GET)

        for route in LAMBDA_ROUTES {
            if *method == route.method {
                if let Some(params) = match_path(path, route.path_pattern) {
                    return Ok((route.operation, params));
                }
            }
        }

        Err(LambdaError::unknown_operation(method, path))
    }
}

/// Extracted path parameters from URL matching.
#[derive(Debug, Default)]
pub struct PathParams {
    pub entries: Vec<(String, String)>,
}

impl PathParams {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Match a URL path against a pattern with `{param}` placeholders.
///
/// Returns extracted parameters if the pattern matches.
fn match_path(path: &str, pattern: &str) -> Option<PathParams> {
    let path_segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let pattern_segments: Vec<&str> = pattern.trim_matches('/').split('/').collect();

    if path_segments.len() != pattern_segments.len() {
        return None;
    }

    let mut params = PathParams::default();
    for (ps, pp) in path_segments.iter().zip(pattern_segments.iter()) {
        if pp.starts_with('{') && pp.ends_with('}') {
            let name = &pp[1..pp.len() - 1];
            params.entries.push((name.to_string(), (*ps).to_string()));
        } else if ps != pp {
            return None;
        }
    }
    Some(params)
}
```

### 8.2 Request Deserialization

For `restJson1`, request parameters are scattered across multiple HTTP locations. The deserialization logic must:

1. Extract path labels from the URL
2. Extract query parameters from the query string
3. Extract header values from request headers
4. Deserialize the remaining JSON body

```rust
/// Deserialize a Lambda request from its restJson1 components.
pub trait FromRestJson1: Sized {
    /// Deserialize from path params, query string, headers, and JSON body.
    fn from_request(
        path_params: &PathParams,
        query: &str,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<Self, LambdaError>;
}
```

For `Invoke`, the body is the raw payload (not a JSON struct wrapping it) because the Smithy model uses `@httpPayload` on the `Payload` member. This means the body bytes are passed directly as the invocation event, not deserialized into a struct.

### 8.3 Response Serialization

```rust
/// Serialize a Lambda response to restJson1 format.
pub trait IntoRestJson1Response {
    /// Serialize to an HTTP response with proper status code, headers, and body.
    fn into_response(self, success_status: u16) -> Result<http::Response<Bytes>, LambdaError>;
}
```

For `Invoke`, the response body is the raw function output (via `@httpPayload`), and additional fields are bound to response headers:
- `X-Amz-Function-Error` -- set if the function returned an error
- `X-Amz-Log-Result` -- base64-encoded last 4KB of logs (if `LogType: Tail`)
- `X-Amz-Executed-Version` -- the version that was actually invoked

### 8.4 LambdaHandler Trait

```rust
/// The boundary between HTTP and business logic.
///
/// Unlike the awsJson services which receive a single typed input,
/// the Lambda handler receives pre-parsed request components and
/// returns a response that includes status code, headers, and body.
pub trait LambdaHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: LambdaOperation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, LambdaError>> + Send>>;
}
```

### 8.5 Service Integration

```rust
/// Hyper Service implementation for Lambda.
pub struct LambdaHttpService<H> {
    handler: Arc<H>,
    config: LambdaHttpConfig,
}

pub struct LambdaHttpConfig {
    pub skip_signature_validation: bool,
    pub region: String,
    pub account_id: String,
}
```

---

## 9. Execution Engine Design

The execution engine is the most complex and Lambda-specific component. It manages Docker containers that run function code.

### 9.1 Execution Flow

```
    Invoke request arrives
            |
            v
    +------------------+
    | Resolve function |  Look up function name/qualifier -> VersionRecord
    | + version        |
    +--------+---------+
             |
             v
    +------------------+
    | Check warm pool  |  Is there a warm container for this version?
    +--------+---------+
            / \
           /   \
       yes/     \no
         /       \
        v         v
  +---------+  +------------------+
  | Reuse   |  | Start container  |  Pull image if needed,
  | warm    |  | (cold start)     |  docker create + start
  +---------+  +------------------+
        \         /
         \       /
          v     v
    +------------------+
    | Forward payload  |  POST to container's RIE endpoint
    | to container RIE |  http://container:8080/2015-03-31/
    +--------+---------+  functions/function/invocations
             |
             v
    +------------------+
    | Collect response |  Read response body + logs
    +--------+---------+
             |
             v
    +------------------+
    | Return to warm   |  Keep container for next invocation
    | pool (or stop)   |  (with configurable idle timeout)
    +------------------+
```

### 9.2 ExecutionEngine Trait

```rust
/// Abstraction over function execution backends.
///
/// The primary implementation is `DockerExecutionEngine` which runs
/// functions in Docker containers. The trait exists to allow a
/// `NoopExecutionEngine` for `DryRun` invocations and testing.
#[async_trait::async_trait]
pub trait ExecutionEngine: Send + Sync + 'static {
    /// Invoke a function and return the response payload and metadata.
    async fn invoke(
        &self,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, LambdaServiceError>;

    /// Pre-pull the Docker image for a runtime (optional optimization).
    async fn prepare(&self, runtime_image: &str) -> Result<(), LambdaServiceError>;

    /// Shut down all containers managed by this engine.
    async fn shutdown(&self) -> Result<(), LambdaServiceError>;
}

/// Request to invoke a function in a container.
pub struct InvokeRequest {
    /// Unique identifier for this function version (used for container pooling).
    pub function_id: String,
    /// Docker image to use (e.g., "public.ecr.aws/lambda/python:3.12").
    pub runtime_image: String,
    /// Path to extracted function code on the host filesystem.
    pub code_path: PathBuf,
    /// Function handler (e.g., "index.handler").
    pub handler: String,
    /// Environment variables to pass to the container.
    pub environment: HashMap<String, String>,
    /// Invocation payload (JSON bytes).
    pub payload: Bytes,
    /// Function timeout in seconds.
    pub timeout_seconds: u32,
    /// Function memory size in MB (used for container memory limit).
    pub memory_mb: u32,
    /// Whether to capture logs for LogResult header.
    pub capture_logs: bool,
}

/// Response from a function invocation.
pub struct InvokeResponse {
    /// Response payload (JSON bytes).
    pub payload: Bytes,
    /// Function error type, if the function returned an error.
    pub function_error: Option<String>,
    /// Last 4KB of container logs (base64-encoded), if requested.
    pub log_result: Option<String>,
    /// Whether this was a cold start.
    pub cold_start: bool,
}
```

### 9.3 DockerExecutionEngine

```rust
/// Docker-based execution engine using bollard.
///
/// Manages container lifecycle:
/// 1. Image pulling (on first use of a runtime)
/// 2. Container creation with function code mounted
/// 3. Invocation via HTTP POST to container RIE
/// 4. Warm container pooling for reuse
/// 5. Idle container cleanup
pub struct DockerExecutionEngine {
    /// Docker client (via bollard).
    docker: bollard::Docker,
    /// Pool of warm containers keyed by function_id.
    warm_pool: DashMap<String, Vec<WarmContainer>>,
    /// Configuration.
    config: ExecutionConfig,
    /// Set of images already pulled (avoid redundant pulls).
    pulled_images: DashMap<String, ()>,
    /// Network name for Lambda containers.
    network_name: String,
}

pub struct ExecutionConfig {
    /// Maximum number of warm containers per function version.
    pub max_warm_containers: usize,
    /// Idle timeout before a warm container is stopped.
    pub warm_container_idle_timeout: Duration,
    /// Docker socket path (default: /var/run/docker.sock).
    pub docker_socket: String,
    /// Network mode for containers (default: bridge).
    pub network_mode: String,
    /// Host address that containers can use to reach the Rustack gateway.
    /// Used for setting AWS_ENDPOINT_URL inside containers.
    pub host_gateway_address: String,
}

struct WarmContainer {
    /// Docker container ID.
    container_id: String,
    /// The RIE endpoint (http://container_ip:8080).
    endpoint: String,
    /// Last invocation time (for idle timeout).
    last_used: Instant,
    /// Whether this container is currently processing a request.
    in_use: AtomicBool,
}
```

### 9.4 Container Lifecycle

#### 9.4.1 Image Selection

For zip-based deployments, the runtime determines the Docker image:

```rust
/// Map Lambda runtime identifier to Docker base image.
fn runtime_to_image(runtime: &str) -> Result<String, LambdaServiceError> {
    let image = match runtime {
        "python3.9" => "public.ecr.aws/lambda/python:3.9",
        "python3.10" => "public.ecr.aws/lambda/python:3.10",
        "python3.11" => "public.ecr.aws/lambda/python:3.11",
        "python3.12" => "public.ecr.aws/lambda/python:3.12",
        "python3.13" => "public.ecr.aws/lambda/python:3.13",
        "nodejs18.x" => "public.ecr.aws/lambda/nodejs:18",
        "nodejs20.x" => "public.ecr.aws/lambda/nodejs:20",
        "nodejs22.x" => "public.ecr.aws/lambda/nodejs:22",
        "java21" => "public.ecr.aws/lambda/java:21",
        "java17" => "public.ecr.aws/lambda/java:17",
        "dotnet8" => "public.ecr.aws/lambda/dotnet:8",
        "ruby3.3" => "public.ecr.aws/lambda/ruby:3.3",
        "ruby3.4" => "public.ecr.aws/lambda/ruby:3.4",
        "provided.al2023" => "public.ecr.aws/lambda/provided:al2023",
        "provided.al2" => "public.ecr.aws/lambda/provided:al2",
        _ => return Err(LambdaServiceError::InvalidRuntime {
            runtime: runtime.to_string(),
        }),
    };
    Ok(image.to_string())
}
```

For container image deployments (`PackageType: Image`), the `ImageUri` is used directly.

#### 9.4.2 Container Creation

```rust
impl DockerExecutionEngine {
    async fn create_container(
        &self,
        request: &InvokeRequest,
    ) -> Result<WarmContainer, LambdaServiceError> {
        // 1. Pull image if not already pulled.
        self.ensure_image(&request.runtime_image).await?;

        // 2. Build container configuration.
        let mut env_vars: Vec<String> = request.environment.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        // Add Lambda-standard environment variables.
        env_vars.push(format!(
            "AWS_LAMBDA_FUNCTION_NAME={}",
            request.function_id
        ));
        env_vars.push(format!(
            "AWS_LAMBDA_FUNCTION_MEMORY_SIZE={}",
            request.memory_mb
        ));
        env_vars.push(format!(
            "AWS_LAMBDA_FUNCTION_TIMEOUT={}",
            request.timeout_seconds
        ));
        env_vars.push(format!("_HANDLER={}", request.handler));

        let host_config = bollard::models::HostConfig {
            binds: Some(vec![
                format!("{}:/var/task:ro", request.code_path.display()),
            ]),
            memory: Some((request.memory_mb as i64) * 1024 * 1024),
            network_mode: Some(self.config.network_mode.clone()),
            ..Default::default()
        };

        let config = bollard::container::Config {
            image: Some(request.runtime_image.clone()),
            env: Some(env_vars),
            host_config: Some(host_config),
            ..Default::default()
        };

        // 3. Create and start container.
        let create_resp = self.docker
            .create_container::<String, String>(None, config)
            .await
            .map_err(|e| LambdaServiceError::DockerError {
                message: format!("failed to create container: {e}"),
            })?;

        let container_id = create_resp.id;

        self.docker
            .start_container::<String>(&container_id, None)
            .await
            .map_err(|e| LambdaServiceError::DockerError {
                message: format!("failed to start container: {e}"),
            })?;

        // 4. Get container IP address.
        let inspect = self.docker
            .inspect_container(&container_id, None)
            .await
            .map_err(|e| LambdaServiceError::DockerError {
                message: format!("failed to inspect container: {e}"),
            })?;

        let ip = inspect
            .network_settings
            .and_then(|ns| ns.ip_address)
            .ok_or_else(|| LambdaServiceError::DockerError {
                message: "container has no IP address".to_string(),
            })?;

        let endpoint = format!("http://{ip}:8080");

        // 5. Wait for RIE to be ready (poll health).
        self.wait_for_rie_ready(&endpoint).await?;

        Ok(WarmContainer {
            container_id,
            endpoint,
            last_used: Instant::now(),
            in_use: AtomicBool::new(false),
        })
    }

    /// Poll the RIE endpoint until it responds or timeout.
    async fn wait_for_rie_ready(&self, endpoint: &str) -> Result<(), LambdaServiceError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let client = reqwest::Client::new();
        loop {
            if Instant::now() > deadline {
                return Err(LambdaServiceError::ContainerStartTimeout);
            }
            // RIE does not have a health endpoint; we try a HEAD request.
            // Any response (even 4xx) means it is up.
            match client.head(endpoint).send().await {
                Ok(_) => return Ok(()),
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
}
```

#### 9.4.3 Invocation

```rust
impl DockerExecutionEngine {
    async fn invoke_container(
        &self,
        container: &WarmContainer,
        request: &InvokeRequest,
    ) -> Result<InvokeResponse, LambdaServiceError> {
        let url = format!(
            "{}/2015-03-31/functions/function/invocations",
            container.endpoint
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(request.payload.clone())
            .timeout(Duration::from_secs(request.timeout_seconds as u64))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LambdaServiceError::FunctionTimeout {
                        timeout_seconds: request.timeout_seconds,
                    }
                } else {
                    LambdaServiceError::InvocationError {
                        message: format!("failed to invoke function: {e}"),
                    }
                }
            })?;

        let function_error = resp
            .headers()
            .get("X-Amz-Function-Error")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let payload = resp.bytes().await.map_err(|e| {
            LambdaServiceError::InvocationError {
                message: format!("failed to read response: {e}"),
            }
        })?;

        // Capture logs if requested.
        let log_result = if request.capture_logs {
            self.capture_container_logs(&container.container_id).await.ok()
        } else {
            None
        };

        Ok(InvokeResponse {
            payload: Bytes::from(payload),
            function_error,
            log_result,
            cold_start: false, // caller sets this based on whether container was new
        })
    }

    /// Capture the last 4KB of container logs, base64-encoded.
    async fn capture_container_logs(
        &self,
        container_id: &str,
    ) -> Result<String, LambdaServiceError> {
        use bollard::container::LogsOptions;
        use futures_util::StreamExt;

        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: "100".to_string(),
            ..Default::default()
        };

        let mut log_stream = self.docker.logs(container_id, Some(options));
        let mut log_bytes = Vec::new();

        while let Some(Ok(output)) = log_stream.next().await {
            log_bytes.extend_from_slice(&output.into_bytes());
        }

        // Keep last 4KB.
        let start = log_bytes.len().saturating_sub(4096);
        let truncated = &log_bytes[start..];
        Ok(base64::engine::general_purpose::STANDARD.encode(truncated))
    }
}
```

### 9.5 Warm Container Pool

The container pool keeps containers alive between invocations to avoid cold starts:

```rust
impl DockerExecutionEngine {
    /// Get or create a container for the given function.
    async fn get_container(
        &self,
        request: &InvokeRequest,
    ) -> Result<(WarmContainer, bool), LambdaServiceError> {
        // Try to acquire a warm container.
        if let Some(mut containers) = self.warm_pool.get_mut(&request.function_id) {
            for container in containers.iter_mut() {
                if !container.in_use.swap(true, Ordering::AcqRel) {
                    container.last_used = Instant::now();
                    // Clone the container info for use outside the lock.
                    return Ok((container.clone(), false)); // false = not a cold start
                }
            }
        }

        // No warm container available; create a new one (cold start).
        let container = self.create_container(request).await?;
        Ok((container, true)) // true = cold start
    }

    /// Return a container to the warm pool after invocation.
    fn return_to_pool(&self, function_id: &str, container: WarmContainer) {
        let mut entry = self.warm_pool.entry(function_id.to_string()).or_default();
        if entry.len() < self.config.max_warm_containers {
            container.in_use.store(false, Ordering::Release);
            entry.push(container);
        } else {
            // Pool is full; stop the container.
            let docker = self.docker.clone();
            let container_id = container.container_id.clone();
            tokio::spawn(async move {
                let _ = docker
                    .stop_container(&container_id, None)
                    .await;
                let _ = docker
                    .remove_container(&container_id, None)
                    .await;
            });
        }
    }
}
```

### 9.6 Idle Container Cleanup

A background task periodically stops containers that have been idle beyond the timeout:

```rust
impl DockerExecutionEngine {
    /// Start the background cleanup task.
    pub fn start_cleanup_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                engine.cleanup_idle_containers().await;
            }
        })
    }

    async fn cleanup_idle_containers(&self) {
        let now = Instant::now();
        let timeout = self.config.warm_container_idle_timeout;

        let mut to_remove: Vec<(String, String)> = Vec::new();

        for mut entry in self.warm_pool.iter_mut() {
            entry.value_mut().retain(|c| {
                if !c.in_use.load(Ordering::Acquire)
                    && now.duration_since(c.last_used) > timeout
                {
                    to_remove.push((
                        entry.key().clone(),
                        c.container_id.clone(),
                    ));
                    false
                } else {
                    true
                }
            });
        }

        // Stop removed containers asynchronously.
        for (_, container_id) in to_remove {
            let docker = self.docker.clone();
            tokio::spawn(async move {
                let _ = docker.stop_container(&container_id, None).await;
                let _ = docker.remove_container(&container_id, None).await;
            });
        }
    }
}
```

### 9.7 Container Image Deployment

For functions with `PackageType: Image`, the `ImageUri` is used directly as the Docker image. No code mounting is needed -- the container image already contains the function code and runtime.

```rust
impl DockerExecutionEngine {
    async fn create_image_container(
        &self,
        request: &InvokeRequest,
    ) -> Result<WarmContainer, LambdaServiceError> {
        self.ensure_image(&request.runtime_image).await?;

        let env_vars: Vec<String> = request.environment.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let host_config = bollard::models::HostConfig {
            // No binds -- code is baked into the image.
            memory: Some((request.memory_mb as i64) * 1024 * 1024),
            network_mode: Some(self.config.network_mode.clone()),
            ..Default::default()
        };

        let config = bollard::container::Config {
            image: Some(request.runtime_image.clone()),
            env: Some(env_vars),
            host_config: Some(host_config),
            ..Default::default()
        };

        // ... same create + start + inspect flow as zip-based
    }
}
```

---

## 10. Storage Engine Design

### 10.1 Overview

The storage engine manages function metadata, code artifacts, versions, aliases, permissions, and tags. Unlike the execution engine (which manages Docker containers), the storage engine is purely in-memory data structures.

### 10.2 Core Data Structures

```rust
/// Top-level function store.
/// Keyed by (account_id, region) via rustack-core, then by function name.
pub struct FunctionStore {
    /// All functions keyed by function name.
    functions: DashMap<String, FunctionRecord>,
    /// Temporary directory for extracted function code.
    code_dir: PathBuf,
}

/// A single Lambda function with its versions and aliases.
pub struct FunctionRecord {
    /// Function name.
    pub name: String,
    /// Function ARN (without qualifier).
    pub arn: String,
    /// The unpublished version ($LATEST) configuration.
    pub latest: VersionRecord,
    /// Published versions: version_number -> VersionRecord.
    /// Version numbers are 1-indexed and monotonically increasing.
    pub versions: BTreeMap<u64, VersionRecord>,
    /// Next version number to assign.
    pub next_version: u64,
    /// Aliases: alias_name -> AliasRecord.
    pub aliases: HashMap<String, AliasRecord>,
    /// Resource-based policy statements.
    pub policy: PolicyDocument,
    /// Tags on the function.
    pub tags: HashMap<String, String>,
    /// Function URL configuration (if any).
    pub url_config: Option<FunctionUrlConfigRecord>,
    /// Creation timestamp (ISO-8601).
    pub created_at: String,
}

/// A single version of a function (either $LATEST or a published version).
pub struct VersionRecord {
    /// Version identifier: "$LATEST" or numeric string ("1", "2", ...).
    pub version: String,
    /// Runtime (e.g., "python3.12", "nodejs20.x").
    pub runtime: Option<String>,
    /// Handler (e.g., "index.handler").
    pub handler: Option<String>,
    /// IAM role ARN.
    pub role: String,
    /// Description.
    pub description: String,
    /// Timeout in seconds (1-900, default 3).
    pub timeout: u32,
    /// Memory size in MB (128-10240, default 128).
    pub memory_size: u32,
    /// Environment variables.
    pub environment: HashMap<String, String>,
    /// Package type: Zip or Image.
    pub package_type: PackageType,
    /// Code metadata.
    pub code: CodeRecord,
    /// Function state.
    pub state: FunctionState,
    /// Last modified timestamp (ISO-8601).
    pub last_modified: String,
    /// Architectures (default: [x86_64]).
    pub architectures: Vec<String>,
    /// Ephemeral storage size in MB (default: 512).
    pub ephemeral_storage_size: u32,
    /// SHA-256 hash of the deployment package.
    pub code_sha256: String,
    /// Size of the deployment package in bytes.
    pub code_size: u64,
    /// Revision ID for optimistic concurrency.
    pub revision_id: String,
    /// Image config (for container image deployments).
    pub image_config: Option<ImageConfig>,
    /// Dead letter config.
    pub dead_letter_config: Option<DeadLetterConfig>,
    /// Tracing config (stored, not enforced).
    pub tracing_config: TracingConfig,
    /// VPC config (stored, not enforced).
    pub vpc_config: Option<VpcConfig>,
    /// Layers (stored as ARNs, not downloaded).
    pub layers: Vec<String>,
    /// Logging config.
    pub logging_config: Option<LoggingConfig>,
}

/// Code artifact storage.
pub struct CodeRecord {
    /// For Zip deployments: path to extracted code directory on disk.
    pub code_path: Option<PathBuf>,
    /// For Image deployments: Docker image URI.
    pub image_uri: Option<String>,
    /// Original zip bytes (retained for GetFunction code download URL simulation).
    pub zip_bytes: Option<Bytes>,
}

/// Alias pointing to a specific version.
pub struct AliasRecord {
    /// Alias name.
    pub name: String,
    /// Target function version number (as string, e.g., "1").
    pub function_version: String,
    /// Description.
    pub description: String,
    /// Routing config for weighted aliases (optional).
    pub routing_config: Option<AliasRoutingConfig>,
    /// Revision ID.
    pub revision_id: String,
}

/// Weighted routing between two versions.
pub struct AliasRoutingConfig {
    /// Additional version weights: version -> weight (0.0 to 1.0).
    pub additional_version_weights: HashMap<String, f64>,
}

/// Function URL configuration.
pub struct FunctionUrlConfigRecord {
    /// Auth type: NONE or AWS_IAM.
    pub auth_type: String,
    /// CORS configuration.
    pub cors: Option<CorsConfig>,
    /// Invoke mode: BUFFERED or RESPONSE_STREAM.
    pub invoke_mode: String,
    /// The generated function URL.
    pub function_url: String,
    /// Creation timestamp.
    pub creation_time: String,
    /// Last modified timestamp.
    pub last_modified_time: String,
}

/// Function state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionState {
    /// Function is being created/updated (transitional).
    Pending,
    /// Function is ready to invoke.
    Active,
    /// Function failed to create/update.
    Failed { reason: String },
    /// Function is inactive (no recent invocations).
    Inactive,
}

/// Resource-based policy document.
pub struct PolicyDocument {
    pub statements: Vec<PolicyStatement>,
}

pub struct PolicyStatement {
    pub sid: String,
    pub effect: String,
    pub principal: serde_json::Value,
    pub action: String,
    pub resource: String,
    pub condition: Option<serde_json::Value>,
}
```

### 10.3 Code Storage

Function code is stored as extracted zip files on the host filesystem:

```rust
impl FunctionStore {
    /// Store function code from a zip file.
    fn store_zip_code(
        &self,
        function_name: &str,
        version: &str,
        zip_bytes: &[u8],
    ) -> Result<CodeRecord, LambdaServiceError> {
        // Create a directory for this function version's code.
        let code_dir = self.code_dir
            .join(function_name)
            .join(version);
        std::fs::create_dir_all(&code_dir)
            .map_err(|e| LambdaServiceError::Internal {
                message: format!("failed to create code directory: {e}"),
            })?;

        // Extract zip contents.
        let cursor = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| LambdaServiceError::InvalidZipFile {
                message: format!("invalid zip file: {e}"),
            })?;
        archive.extract(&code_dir)
            .map_err(|e| LambdaServiceError::InvalidZipFile {
                message: format!("failed to extract zip: {e}"),
            })?;

        // Compute SHA-256 of the zip.
        let sha256 = compute_sha256(zip_bytes);
        let code_size = zip_bytes.len() as u64;

        Ok(CodeRecord {
            code_path: Some(code_dir),
            image_uri: None,
            zip_bytes: Some(Bytes::copy_from_slice(zip_bytes)),
        })
    }
}
```

### 10.4 Function Name and ARN Resolution

```rust
/// Resolve a function reference to (function_name, qualifier).
///
/// Accepts:
///   "my-function"                                    -> ("my-function", None)
///   "my-function:v1"                                 -> ("my-function", Some("v1"))
///   "arn:aws:lambda:us-east-1:000000000000:function:my-function"       -> ("my-function", None)
///   "arn:aws:lambda:us-east-1:000000000000:function:my-function:v1"    -> ("my-function", Some("v1"))
///   "000000000000:function:my-function"              -> ("my-function", None)
pub fn resolve_function_ref(
    function_ref: &str,
) -> Result<(String, Option<String>), LambdaServiceError> {
    if function_ref.starts_with("arn:aws:lambda:") {
        // Full ARN: arn:aws:lambda:region:account:function:name[:qualifier]
        let parts: Vec<&str> = function_ref.split(':').collect();
        if parts.len() >= 7 {
            let name = parts[6].to_string();
            let qualifier = parts.get(7).map(|s| s.to_string());
            return Ok((name, qualifier));
        }
        return Err(LambdaServiceError::InvalidArn {
            arn: function_ref.to_string(),
        });
    }

    if function_ref.contains(":function:") {
        // Partial ARN: account:function:name[:qualifier]
        let parts: Vec<&str> = function_ref.split(':').collect();
        if parts.len() >= 3 {
            let name = parts[2].to_string();
            let qualifier = parts.get(3).map(|s| s.to_string());
            return Ok((name, qualifier));
        }
    }

    // Plain name or name:qualifier
    if let Some((name, qualifier)) = function_ref.split_once(':') {
        Ok((name.to_string(), Some(qualifier.to_string())))
    } else {
        Ok((function_ref.to_string(), None))
    }
}

/// Resolve a qualifier to a specific VersionRecord.
///
/// - None or "$LATEST" -> latest (unpublished) version
/// - Numeric string -> published version by number
/// - Non-numeric string -> alias -> published version
pub fn resolve_version<'a>(
    function: &'a FunctionRecord,
    qualifier: Option<&str>,
) -> Result<&'a VersionRecord, LambdaServiceError> {
    match qualifier {
        None | Some("$LATEST") => Ok(&function.latest),
        Some(q) => {
            // Try numeric version first.
            if let Ok(version_num) = q.parse::<u64>() {
                function.versions.get(&version_num)
                    .ok_or_else(|| LambdaServiceError::VersionNotFound {
                        function_name: function.name.clone(),
                        version: q.to_string(),
                    })
            } else {
                // Try alias.
                let alias = function.aliases.get(q)
                    .ok_or_else(|| LambdaServiceError::AliasNotFound {
                        function_name: function.name.clone(),
                        alias: q.to_string(),
                    })?;
                let version_num: u64 = alias.function_version.parse()
                    .map_err(|_| LambdaServiceError::Internal {
                        message: format!("alias {} points to invalid version", q),
                    })?;
                function.versions.get(&version_num)
                    .ok_or_else(|| LambdaServiceError::VersionNotFound {
                        function_name: function.name.clone(),
                        version: alias.function_version.clone(),
                    })
            }
        }
    }
}
```

### 10.5 ARN Construction

```rust
fn function_arn(region: &str, account_id: &str, function_name: &str) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}")
}

fn function_version_arn(
    region: &str,
    account_id: &str,
    function_name: &str,
    version: &str,
) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}:{version}")
}

fn alias_arn(
    region: &str,
    account_id: &str,
    function_name: &str,
    alias_name: &str,
) -> String {
    format!("arn:aws:lambda:{region}:{account_id}:function:{function_name}:{alias_name}")
}
```

---

## 11. Core Business Logic

### 11.1 Provider

```rust
/// Main Lambda provider. Owns function storage and the execution engine.
pub struct RustackLambda {
    /// Function storage (metadata, code, versions, aliases).
    store: FunctionStore,
    /// Docker execution engine for invoking functions.
    engine: Arc<dyn ExecutionEngine>,
    /// Configuration.
    config: Arc<LambdaConfig>,
    /// Async invocation queue (for Event invocation type).
    async_queue: mpsc::Sender<AsyncInvocation>,
}

pub struct LambdaConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
    /// Docker socket path.
    pub docker_socket: String,
    /// Maximum warm containers per function.
    pub max_warm_containers: usize,
    /// Warm container idle timeout in seconds.
    pub warm_container_idle_seconds: u64,
    /// Whether Docker is available (if false, Invoke returns an error).
    pub docker_enabled: bool,
}
```

### 11.2 Operations Grouped by Category

#### Function Management (7 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateFunction` | 0 | High | Decode zip, store code, set state to Active, optionally publish version |
| `GetFunction` | 0 | Low | Return configuration + code location + tags |
| `GetFunctionConfiguration` | 0 | Low | Return configuration only (no code location) |
| `UpdateFunctionCode` | 0 | Medium | Replace code, update CodeSha256/CodeSize, set state to Active |
| `UpdateFunctionConfiguration` | 0 | Medium | Update handler, runtime, env, timeout, memory, etc. |
| `DeleteFunction` | 0 | Medium | Delete function or specific version, cleanup code files and containers |
| `ListFunctions` | 0 | Low | Paginate with Marker/MaxItems, optionally include all versions |

#### Invocation (1 operation, 3 modes)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `Invoke` (RequestResponse) | 0 | High | Synchronous: start/reuse container, forward payload, return response |
| `Invoke` (Event) | 2 | Medium | Asynchronous: enqueue, return 202 immediately, execute in background |
| `Invoke` (DryRun) | 0 | Low | Validate only, return 204 |

#### Versions (2 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `PublishVersion` | 1 | Medium | Snapshot $LATEST as immutable version, increment version counter |
| `ListVersionsByFunction` | 1 | Low | Return all versions including $LATEST |

#### Aliases (5 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateAlias` | 1 | Low | Point alias to a published version |
| `GetAlias` | 1 | Low | Return alias configuration |
| `UpdateAlias` | 1 | Low | Change target version or routing config |
| `DeleteAlias` | 1 | Low | Remove alias |
| `ListAliases` | 1 | Low | Return all aliases for a function |

#### Permissions (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `AddPermission` | 2 | Low | Store policy statement, no enforcement |
| `RemovePermission` | 2 | Low | Remove statement by Sid |
| `GetPolicy` | 2 | Low | Return JSON policy document |

#### Tags (3 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `TagResource` | 2 | Low | Add/update tags (max 50) |
| `UntagResource` | 2 | Low | Remove tags by key |
| `ListTags` | 2 | Low | Return all tags |

#### Function URLs (5 operations)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `CreateFunctionUrlConfig` | 3 | Medium | Generate URL, store config |
| `GetFunctionUrlConfig` | 3 | Low | Return config |
| `UpdateFunctionUrlConfig` | 3 | Low | Update auth type, CORS |
| `DeleteFunctionUrlConfig` | 3 | Low | Remove config |
| `ListFunctionUrlConfigs` | 3 | Low | Return URL configs |

#### Account (1 operation)

| Operation | Phase | Complexity | Notes |
|-----------|-------|------------|-------|
| `GetAccountSettings` | 2 | Low | Return static account limits |

### 11.3 CreateFunction Logic

```rust
impl RustackLambda {
    pub async fn create_function(
        &self,
        input: CreateFunctionInput,
    ) -> Result<FunctionConfiguration, LambdaServiceError> {
        let function_name = &input.function_name;

        // Validate function name.
        validate_function_name(function_name)?;

        // Check for conflict.
        if self.store.functions.contains_key(function_name) {
            return Err(LambdaServiceError::ResourceConflict {
                message: format!(
                    "Function already exist: {}",
                    function_name
                ),
            });
        }

        // Determine package type and process code.
        let package_type = input.package_type
            .as_deref()
            .unwrap_or("Zip");

        let (code, code_sha256, code_size) = match package_type {
            "Zip" => {
                let zip_bytes = input.code.zip_file
                    .as_ref()
                    .ok_or(LambdaServiceError::InvalidParameter {
                        message: "ZipFile is required for Zip package type".to_string(),
                    })?;

                // Decode base64 zip.
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(zip_bytes)
                    .map_err(|e| LambdaServiceError::InvalidZipFile {
                        message: format!("invalid base64: {e}"),
                    })?;

                let sha256 = compute_sha256(&decoded);
                let size = decoded.len() as u64;
                let code = self.store.store_zip_code(
                    function_name, "$LATEST", &decoded,
                )?;
                (code, sha256, size)
            }
            "Image" => {
                let image_uri = input.code.image_uri
                    .as_ref()
                    .ok_or(LambdaServiceError::InvalidParameter {
                        message: "ImageUri is required for Image package type".to_string(),
                    })?;
                let code = CodeRecord {
                    code_path: None,
                    image_uri: Some(image_uri.clone()),
                    zip_bytes: None,
                };
                let sha256 = compute_sha256(image_uri.as_bytes());
                (code, sha256, 0)
            }
            _ => return Err(LambdaServiceError::InvalidParameter {
                message: format!("invalid PackageType: {package_type}"),
            }),
        };

        // Build version record.
        let now = chrono::Utc::now().to_rfc3339();
        let revision_id = uuid::Uuid::new_v4().to_string();
        let arn = function_arn(
            &self.config.default_region,
            &self.config.account_id,
            function_name,
        );

        let version_record = VersionRecord {
            version: "$LATEST".to_string(),
            runtime: input.runtime.clone(),
            handler: input.handler.clone(),
            role: input.role.clone(),
            description: input.description.clone().unwrap_or_default(),
            timeout: input.timeout.unwrap_or(3),
            memory_size: input.memory_size.unwrap_or(128),
            environment: input.environment
                .as_ref()
                .and_then(|e| e.variables.clone())
                .unwrap_or_default(),
            package_type: if package_type == "Image" {
                PackageType::Image
            } else {
                PackageType::Zip
            },
            code,
            state: FunctionState::Active,
            last_modified: now.clone(),
            architectures: input.architectures
                .unwrap_or_else(|| vec!["x86_64".to_string()]),
            ephemeral_storage_size: input.ephemeral_storage
                .as_ref()
                .map(|e| e.size)
                .unwrap_or(512),
            code_sha256: code_sha256.clone(),
            code_size,
            revision_id,
            image_config: input.image_config,
            dead_letter_config: input.dead_letter_config,
            tracing_config: input.tracing_config
                .unwrap_or(TracingConfig { mode: "PassThrough".to_string() }),
            vpc_config: input.vpc_config,
            layers: input.layers.unwrap_or_default(),
            logging_config: input.logging_config,
        };

        let function = FunctionRecord {
            name: function_name.clone(),
            arn: arn.clone(),
            latest: version_record,
            versions: BTreeMap::new(),
            next_version: 1,
            aliases: HashMap::new(),
            policy: PolicyDocument { statements: Vec::new() },
            tags: input.tags.unwrap_or_default(),
            url_config: None,
            created_at: now,
        };

        // Optionally publish version 1.
        let mut function = function;
        if input.publish.unwrap_or(false) {
            self.publish_version_internal(&mut function)?;
        }

        // Pre-pull the runtime image if Docker is enabled.
        if self.config.docker_enabled {
            if let Some(ref runtime) = function.latest.runtime {
                let image = runtime_to_image(runtime)?;
                let engine = Arc::clone(&self.engine);
                tokio::spawn(async move {
                    let _ = engine.prepare(&image).await;
                });
            }
        }

        let config = self.build_function_configuration(&function, None);
        self.store.functions.insert(function_name.clone(), function);
        Ok(config)
    }
}
```

### 11.4 Invoke Logic

```rust
impl RustackLambda {
    pub async fn invoke(
        &self,
        function_ref: &str,
        qualifier: Option<&str>,
        invocation_type: &str,
        payload: Bytes,
        log_type: Option<&str>,
    ) -> Result<InvokeOutput, LambdaServiceError> {
        let (function_name, ref_qualifier) = resolve_function_ref(function_ref)?;
        let effective_qualifier = qualifier
            .or(ref_qualifier.as_deref());

        let function = self.store.functions.get(&function_name)
            .ok_or_else(|| LambdaServiceError::FunctionNotFound {
                name: function_name.clone(),
            })?;

        let version = resolve_version(&function, effective_qualifier)?;

        // Check function state.
        if version.state != FunctionState::Active {
            return Err(LambdaServiceError::ResourceNotReady {
                message: format!(
                    "Function {} is in {} state",
                    function_name,
                    version.state
                ),
            });
        }

        match invocation_type {
            "DryRun" => {
                Ok(InvokeOutput {
                    status_code: 204,
                    payload: Bytes::new(),
                    function_error: None,
                    log_result: None,
                    executed_version: version.version.clone(),
                })
            }
            "Event" => {
                // Enqueue for async execution.
                let invocation = AsyncInvocation {
                    function_name: function_name.clone(),
                    qualifier: effective_qualifier.map(String::from),
                    payload,
                };
                self.async_queue.send(invocation).await
                    .map_err(|_| LambdaServiceError::Internal {
                        message: "async queue full".to_string(),
                    })?;
                Ok(InvokeOutput {
                    status_code: 202,
                    payload: Bytes::new(),
                    function_error: None,
                    log_result: None,
                    executed_version: version.version.clone(),
                })
            }
            "RequestResponse" | "" => {
                if !self.config.docker_enabled {
                    return Err(LambdaServiceError::DockerNotAvailable);
                }

                // Build the execution request.
                let runtime_image = match version.package_type {
                    PackageType::Zip => {
                        let runtime = version.runtime.as_deref()
                            .ok_or(LambdaServiceError::InvalidParameter {
                                message: "runtime is required for Zip functions".to_string(),
                            })?;
                        runtime_to_image(runtime)?
                    }
                    PackageType::Image => {
                        version.code.image_uri.clone()
                            .ok_or(LambdaServiceError::InvalidParameter {
                                message: "ImageUri is required for Image functions".to_string(),
                            })?
                    }
                };

                let function_id = format!("{}-{}", function_name, version.version);
                let request = InvokeRequest {
                    function_id,
                    runtime_image,
                    code_path: version.code.code_path.clone()
                        .unwrap_or_default(),
                    handler: version.handler.clone().unwrap_or_default(),
                    environment: version.environment.clone(),
                    payload,
                    timeout_seconds: version.timeout,
                    memory_mb: version.memory_size,
                    capture_logs: log_type == Some("Tail"),
                };

                let response = self.engine.invoke(request).await?;

                Ok(InvokeOutput {
                    status_code: 200,
                    payload: response.payload,
                    function_error: response.function_error,
                    log_result: response.log_result,
                    executed_version: version.version.clone(),
                })
            }
            _ => Err(LambdaServiceError::InvalidParameter {
                message: format!("invalid InvocationType: {invocation_type}"),
            }),
        }
    }
}
```

### 11.5 PublishVersion Logic

```rust
impl RustackLambda {
    fn publish_version_internal(
        &self,
        function: &mut FunctionRecord,
    ) -> Result<u64, LambdaServiceError> {
        let version_num = function.next_version;
        function.next_version += 1;

        // Clone the $LATEST configuration as an immutable version.
        let mut published = function.latest.clone();
        published.version = version_num.to_string();

        // If $LATEST has code, copy it to a version-specific directory.
        if let Some(ref latest_path) = function.latest.code.code_path {
            let version_path = latest_path.parent()
                .unwrap_or(latest_path)
                .join(&published.version);
            if latest_path.exists() {
                // Copy code directory.
                copy_dir_recursive(latest_path, &version_path)?;
                published.code.code_path = Some(version_path);
            }
        }

        function.versions.insert(version_num, published);
        Ok(version_num)
    }
}
```

### 11.6 Async Invocation Queue

For `Event` invocation type, requests are queued and processed by a background worker:

```rust
struct AsyncInvocation {
    function_name: String,
    qualifier: Option<String>,
    payload: Bytes,
}

impl RustackLambda {
    /// Start the async invocation worker.
    pub fn start_async_worker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let lambda = Arc::clone(self);
        let (tx, mut rx) = mpsc::channel::<AsyncInvocation>(1000);
        // Store tx in self.async_queue

        tokio::spawn(async move {
            while let Some(invocation) = rx.recv().await {
                let lambda = Arc::clone(&lambda);
                // Process each async invocation in a spawned task.
                tokio::spawn(async move {
                    let result = lambda.invoke(
                        &invocation.function_name,
                        invocation.qualifier.as_deref(),
                        "RequestResponse",
                        invocation.payload,
                        None,
                    ).await;
                    if let Err(e) = result {
                        tracing::warn!(
                            function = %invocation.function_name,
                            error = %e,
                            "async invocation failed"
                        );
                    }
                });
            }
        })
    }
}
```

---

## 12. Error Handling

### 12.1 Lambda Error Codes

```rust
/// Lambda service errors with HTTP status codes and error type strings.
#[derive(Debug, thiserror::Error)]
pub enum LambdaServiceError {
    #[error("Function not found: {name}")]
    FunctionNotFound { name: String },

    #[error("Version not found: {function_name}:{version}")]
    VersionNotFound { function_name: String, version: String },

    #[error("Alias not found: {function_name}:{alias}")]
    AliasNotFound { function_name: String, alias: String },

    #[error("Resource already exists: {message}")]
    ResourceConflict { message: String },

    #[error("Invalid parameter: {message}")]
    InvalidParameter { message: String },

    #[error("Invalid zip file: {message}")]
    InvalidZipFile { message: String },

    #[error("Invalid runtime: {runtime}")]
    InvalidRuntime { runtime: String },

    #[error("Invalid ARN: {arn}")]
    InvalidArn { arn: String },

    #[error("Function is not ready: {message}")]
    ResourceNotReady { message: String },

    #[error("Function timed out after {timeout_seconds} seconds")]
    FunctionTimeout { timeout_seconds: u32 },

    #[error("Invocation error: {message}")]
    InvocationError { message: String },

    #[error("Docker error: {message}")]
    DockerError { message: String },

    #[error("Docker is not available. Set LAMBDA_DOCKER_ENABLED=true and ensure Docker is running.")]
    DockerNotAvailable,

    #[error("Container start timeout")]
    ContainerStartTimeout,

    #[error("Request too large: payload exceeds {max_bytes} bytes")]
    RequestTooLarge { max_bytes: usize },

    #[error("Policy statement not found: {sid}")]
    PolicyNotFound { sid: String },

    #[error("Too many requests")]
    TooManyRequests,

    #[error("Internal error: {message}")]
    Internal { message: String },
}
```

### 12.2 Error Mapping

```rust
impl LambdaServiceError {
    /// Map to (HTTP status, error type string, message).
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::FunctionNotFound { .. } =>
                (404, "ResourceNotFoundException", self.to_string()),
            Self::VersionNotFound { .. } =>
                (404, "ResourceNotFoundException", self.to_string()),
            Self::AliasNotFound { .. } =>
                (404, "ResourceNotFoundException", self.to_string()),
            Self::ResourceConflict { .. } =>
                (409, "ResourceConflictException", self.to_string()),
            Self::InvalidParameter { .. } =>
                (400, "InvalidParameterValueException", self.to_string()),
            Self::InvalidZipFile { .. } =>
                (400, "InvalidParameterValueException", self.to_string()),
            Self::InvalidRuntime { .. } =>
                (502, "InvalidRuntimeException", self.to_string()),
            Self::InvalidArn { .. } =>
                (400, "InvalidParameterValueException", self.to_string()),
            Self::ResourceNotReady { .. } =>
                (502, "ResourceNotReadyException", self.to_string()),
            Self::FunctionTimeout { .. } =>
                (200, "", self.to_string()), // Timeout is returned as 200 with FunctionError header
            Self::InvocationError { .. } =>
                (502, "ServiceException", self.to_string()),
            Self::DockerError { .. } =>
                (500, "ServiceException", self.to_string()),
            Self::DockerNotAvailable =>
                (500, "ServiceException", self.to_string()),
            Self::ContainerStartTimeout =>
                (500, "ServiceException", self.to_string()),
            Self::RequestTooLarge { .. } =>
                (413, "RequestTooLargeException", self.to_string()),
            Self::PolicyNotFound { .. } =>
                (404, "ResourceNotFoundException", self.to_string()),
            Self::TooManyRequests =>
                (429, "TooManyRequestsException", self.to_string()),
            Self::Internal { .. } =>
                (500, "ServiceException", self.to_string()),
        }
    }
}
```

### 12.3 Error Response Format

```rust
/// Format a Lambda error response for restJson1 protocol.
fn error_response(error: &LambdaServiceError) -> http::Response<Bytes> {
    let (status, error_type, message) = error.to_error_response();

    let body = serde_json::json!({
        "Type": if status >= 500 { "Service" } else { "User" },
        "Message": message,
    });

    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("X-Amzn-Errortype", error_type)
        .body(Bytes::from(serde_json::to_vec(&body).expect("JSON serialization")))
        .expect("valid error response")
}
```

---

## 13. Server Integration

### 13.1 Lambda ServiceRouter

```rust
/// Routes requests to the Lambda service.
///
/// Matches requests whose URL path starts with /2015-03-31/functions,
/// /2021-10-31/functions, /2015-03-31/tags, or /2015-03-31/account-settings.
pub struct LambdaServiceRouter<H: LambdaHandler> {
    inner: LambdaHttpService<H>,
}

impl<H: LambdaHandler> ServiceRouter for LambdaServiceRouter<H> {
    fn name(&self) -> &'static str { "lambda" }

    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        let path = req.uri().path();
        path.starts_with("/2015-03-31/functions")
            || path.starts_with("/2021-10-31/functions")
            || path.starts_with("/2015-03-31/tags")
            || path.starts_with("/2015-03-31/account-settings")
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
```

### 13.2 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "lambda"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
lambda = ["dep:rustack-lambda-core", "dep:rustack-lambda-http"]
```

### 13.3 Gateway Registration Order

Lambda is registered before S3 (catch-all) but the order relative to DynamoDB/SQS/SSM does not matter because Lambda uses URL path matching while the others use `X-Amz-Target` header matching:

```rust
fn build_gateway(config: &ServerConfig) -> GatewayService {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    #[cfg(feature = "dynamodb")]
    services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

    #[cfg(feature = "sqs")]
    services.push(Box::new(SqsServiceRouter::new(sqs_service)));

    #[cfg(feature = "ssm")]
    services.push(Box::new(SSMServiceRouter::new(ssm_service)));

    #[cfg(feature = "lambda")]
    services.push(Box::new(LambdaServiceRouter::new(lambda_service)));

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 13.4 Configuration

```rust
pub struct LambdaConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub account_id: String,
    /// Gateway host address.
    pub host: String,
    /// Gateway port.
    pub port: u16,
    /// Enable Docker-based execution (default: true).
    pub docker_enabled: bool,
    /// Docker socket path.
    pub docker_socket: String,
    /// Maximum warm containers per function version.
    pub max_warm_containers: usize,
    /// Warm container idle timeout in seconds.
    pub warm_container_idle_seconds: u64,
    /// Network mode for Lambda containers.
    pub docker_network_mode: String,
}

impl LambdaConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("LAMBDA_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
            docker_enabled: env_bool("LAMBDA_DOCKER_ENABLED", true),
            docker_socket: env_str("DOCKER_SOCKET", "/var/run/docker.sock"),
            max_warm_containers: env_usize("LAMBDA_MAX_WARM_CONTAINERS", 5),
            warm_container_idle_seconds: env_u64("LAMBDA_WARM_CONTAINER_IDLE_SECONDS", 600),
            docker_network_mode: env_str("LAMBDA_DOCKER_NETWORK_MODE", "bridge"),
        }
    }
}
```

### 13.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with all services) |
| `LAMBDA_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `LAMBDA_DOCKER_ENABLED` | `true` | Enable Docker-based execution |
| `DOCKER_SOCKET` | `/var/run/docker.sock` | Docker daemon socket |
| `LAMBDA_MAX_WARM_CONTAINERS` | `5` | Max warm containers per function version |
| `LAMBDA_WARM_CONTAINER_IDLE_SECONDS` | `600` | Seconds before idle container is stopped |
| `LAMBDA_DOCKER_NETWORK_MODE` | `bridge` | Docker network mode for containers |
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
        "lambda": "running"
    }
}
```

### 13.7 Docker-in-Docker Consideration

When Rustack itself runs in a Docker container, it needs access to the Docker daemon to create Lambda containers. Two approaches:

1. **Docker socket mount**: Mount the host Docker socket into the Rustack container: `-v /var/run/docker.sock:/var/run/docker.sock`. Lambda containers run as siblings on the host Docker daemon.

2. **Docker-in-Docker (DinD)**: Run Docker inside the Rustack container. Heavier but fully isolated.

The socket mount approach is simpler and is the recommended default. The `DOCKER_SOCKET` environment variable allows customization.

For code volume mounts, when Rustack runs in a container, the code path inside the Rustack container must be translated to the host path for the bind mount to work correctly in the sibling container. This requires the `LAMBDA_CODE_HOST_PATH` environment variable to map container-internal paths to host paths.

---

## 14. Testing Strategy

### 14.1 Unit Tests

Each module tested in isolation:

- **Router**: test URL path matching for all 27 operations, path parameter extraction, ambiguous paths
- **Path parameter extraction**: test `{FunctionName}` extraction from various URL patterns
- **Query parameter extraction**: test `Qualifier`, `Marker`, `MaxItems` parsing
- **Function name resolution**: test plain names, ARNs, partial ARNs, name:qualifier syntax
- **Version resolution**: test `$LATEST`, numeric versions, alias resolution, missing versions
- **Code storage**: test zip extraction, SHA-256 computation, directory management
- **Container pool**: test warm container acquisition, return, idle cleanup
- **Runtime mapping**: test all supported runtimes map to correct images
- **Validation**: function names, handlers, timeouts, memory sizes

### 14.2 Integration Tests with aws-sdk-lambda

```rust
// tests/integration/lambda_tests.rs
#[tokio::test]
#[ignore]
async fn test_lambda_create_get_delete_function() {
    let client = aws_sdk_lambda::Client::new(&config);

    // Create function with inline zip
    let zip_bytes = create_test_zip("handler.py", b"def handler(event, ctx): return event");

    let create = client.create_function()
        .function_name("test-fn")
        .runtime(Runtime::Python312)
        .handler("handler.handler")
        .role("arn:aws:iam::000000000000:role/test-role")
        .code(FunctionCode::builder()
            .zip_file(Blob::new(zip_bytes))
            .build())
        .send().await.unwrap();

    assert_eq!(create.function_name(), Some("test-fn"));
    assert_eq!(create.state(), Some(&State::Active));

    // Get function
    let get = client.get_function()
        .function_name("test-fn")
        .send().await.unwrap();
    assert!(get.configuration().is_some());

    // Delete function
    client.delete_function()
        .function_name("test-fn")
        .send().await.unwrap();

    // Verify deleted
    let err = client.get_function()
        .function_name("test-fn")
        .send().await;
    assert!(err.is_err());
}

#[tokio::test]
#[ignore]
async fn test_lambda_invoke_python() {
    // Requires Docker
    let client = aws_sdk_lambda::Client::new(&config);

    let zip_bytes = create_test_zip(
        "handler.py",
        b"def handler(event, context): return {'statusCode': 200, 'body': event.get('name', 'world')}",
    );

    client.create_function()
        .function_name("echo-fn")
        .runtime(Runtime::Python312)
        .handler("handler.handler")
        .role("arn:aws:iam::000000000000:role/test-role")
        .code(FunctionCode::builder()
            .zip_file(Blob::new(zip_bytes))
            .build())
        .send().await.unwrap();

    let invoke = client.invoke()
        .function_name("echo-fn")
        .payload(Blob::new(br#"{"name": "Rustack"}"#.to_vec()))
        .send().await.unwrap();

    assert_eq!(invoke.status_code(), 200);
    assert!(invoke.function_error().is_none());

    let payload: serde_json::Value = serde_json::from_slice(
        invoke.payload().unwrap().as_ref()
    ).unwrap();
    assert_eq!(payload["body"], "Rustack");
}

#[tokio::test]
#[ignore]
async fn test_lambda_versions_and_aliases() {
    // Create function, publish version, create alias, invoke via alias
}

#[tokio::test]
#[ignore]
async fn test_lambda_invoke_dry_run() {
    // DryRun returns 204 without executing
}

#[tokio::test]
#[ignore]
async fn test_lambda_invoke_event_async() {
    // Event returns 202 immediately
}
```

### 14.3 Third-Party Test Suites

#### 14.3.1 LocalStack Lambda Test Suite (Primary)

The most comprehensive open-source Lambda test suite. Already vendored at `vendors/localstack/tests/aws/services/lambda_/`.

Key test files:

| File | Lines | Coverage |
|------|-------|---------|
| `test_lambda_api.py` | 7,001 | API operations: CRUD, versions, aliases, permissions, tags, function URLs |
| `test_lambda.py` | 3,663 | Invocation behavior: sync/async, timeout, error handling, concurrency |
| `test_lambda_common.py` | 286 | Shared utilities and common test patterns |
| `test_lambda_runtimes.py` | ~500 | Runtime-specific invocation tests (Python, Node.js, Java, etc.) |
| `test_lambda_destinations.py` | ~400 | Invocation destinations (out of scope for MVP but useful later) |

**Adaptation strategy**: Run the Python test suite against Rustack's Lambda endpoint. Focus on `test_lambda_api.py` first (pure API tests that do not require actual invocation), then progress to `test_lambda.py` (invocation tests requiring Docker).

```makefile
test-lambda-localstack-api:
	@cd vendors/localstack && python -m pytest tests/aws/services/lambda_/test_lambda_api.py \
	    --endpoint-url=http://localhost:4566 -v -k "not esm and not layer"

test-lambda-localstack:
	@cd vendors/localstack && python -m pytest tests/aws/services/lambda_/test_lambda.py \
	    --endpoint-url=http://localhost:4566 -v
```

#### 14.3.2 moto Lambda Test Suite (Secondary)

- **Repository**: https://github.com/getmoto/moto
- **Location**: `tests/test_awslambda/test_lambda.py` and `tests/test_awslambda/test_lambda_invoke.py`
- **Language**: Python / boto3
- **Coverage**: Comprehensive function CRUD, invocation mocking, version management, alias routing, permissions, tags, concurrency configuration
- **Adaptation**: moto tests mock Lambda without Docker execution. Many API-level tests (CreateFunction, GetFunction, ListFunctions, etc.) can validate our API compatibility without requiring Docker. The invocation tests use moto's mock execution which differs from real container execution.

#### 14.3.3 SAM CLI Integration Test

SAM CLI provides the most realistic Lambda testing workflow. The `sam local start-lambda` command starts a local Lambda endpoint that accepts `Invoke` calls:

```bash
# Test that SAM CLI can invoke functions against Rustack
sam local invoke MyFunction \
    --endpoint-url http://localhost:4566 \
    --event event.json \
    --template template.yaml
```

This validates end-to-end compatibility with the most widely used Lambda local testing tool.

- **Repository**: https://github.com/aws/aws-sam-cli
- **Documentation**: https://docs.aws.amazon.com/serverless-application-model/latest/developerguide/using-sam-cli-local-start-lambda.html

#### 14.3.4 AWS CLI Smoke Tests

Shell-based end-to-end tests for CI:

```bash
ENDPOINT="--endpoint-url http://localhost:4566"

# Create a simple Python function
cd /tmp && mkdir -p lambda-test && cd lambda-test
echo 'def handler(event, ctx): return {"statusCode": 200}' > handler.py
zip function.zip handler.py

aws lambda create-function $ENDPOINT \
    --function-name test-fn \
    --runtime python3.12 \
    --handler handler.handler \
    --role arn:aws:iam::000000000000:role/test-role \
    --zip-file fileb://function.zip

# Get function
aws lambda get-function $ENDPOINT --function-name test-fn

# List functions
aws lambda list-functions $ENDPOINT

# Invoke (requires Docker)
aws lambda invoke $ENDPOINT \
    --function-name test-fn \
    --payload '{"test": true}' \
    /tmp/response.json
cat /tmp/response.json

# Delete function
aws lambda delete-function $ENDPOINT --function-name test-fn
```

### 14.4 Makefile Targets

```makefile
test-lambda: test-lambda-unit test-lambda-integration

test-lambda-unit:
	@cargo test -p rustack-lambda-model -p rustack-lambda-core -p rustack-lambda-http

test-lambda-integration:
	@cargo test -p integration-tests -- lambda --ignored

test-lambda-cli:
	@./tests/lambda-cli-smoke.sh

test-lambda-localstack:
	@cd vendors/localstack && python -m pytest tests/aws/services/lambda_/ -v
```

---

## 15. Phased Implementation Plan

### Phase 0: MVP (8 Operations -- Function CRUD + Synchronous Invoke)

**Goal**: Create functions with zip deployment, invoke them synchronously in Docker containers, manage function lifecycle.
**Estimated scope**: ~8,000-10,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen Extension for restJson1

- Add `Protocol::RestJson1` variant to codegen
- Implement route metadata generation (method, path pattern, success status)
- Implement field binding annotation generation (`@httpLabel`, `@httpQuery`, `@httpHeader`, `@httpPayload`)
- Add `LambdaServiceConfig` to codegen services
- Download Lambda Smithy model JSON
- Generate `rustack-lambda-model` crate with operations, types, route table

#### Step 0.2: HTTP Layer (restJson1 Router)

- Implement `LambdaRouter` (URL path + method pattern matching)
- Implement `PathParams` extraction from URL
- Implement `FromRestJson1` request deserialization (path + query + headers + body)
- Implement `IntoRestJson1Response` response serialization (status + headers + body)
- Implement error formatting with `X-Amzn-Errortype` header
- Implement `LambdaHttpService` (hyper Service)

#### Step 0.3: Storage Engine

- Implement `FunctionStore` with `DashMap`
- Implement `FunctionRecord`, `VersionRecord`, `CodeRecord`
- Implement zip code extraction and directory management
- Implement function name/ARN resolution
- Implement qualifier resolution (`$LATEST`, version number)
- Implement SHA-256 computation for code packages

#### Step 0.4: Docker Execution Engine

- Implement `DockerExecutionEngine` with bollard
- Implement runtime-to-image mapping for Python 3.12 and Node.js 20 (minimum viable runtimes)
- Implement container creation with code volume mount
- Implement RIE readiness polling
- Implement invocation via HTTP POST to container RIE
- Implement response collection (payload + function error)
- Implement basic warm container pooling (single container per function)

#### Step 0.5: Core Operations (8 ops)

- `CreateFunction` (zip deployment only)
- `GetFunction` / `GetFunctionConfiguration`
- `UpdateFunctionCode` / `UpdateFunctionConfiguration`
- `DeleteFunction`
- `ListFunctions`
- `Invoke` (RequestResponse + DryRun)

#### Step 0.6: Server Integration

- Implement `LambdaServiceRouter` with URL prefix matching
- Add `lambda` cargo feature gate
- Register Lambda before S3 in gateway
- Update health endpoint
- Add environment variable configuration

#### Step 0.7: Testing

- Unit tests for router, path matching, function resolution
- Integration tests with aws-sdk-lambda (CRUD + invoke with Docker)
- CLI smoke tests
- Update Makefile with Lambda test targets

### Phase 1: Versions, Aliases, Container Image Support

**Goal**: Publish immutable function versions, create aliases pointing to versions, support container image deployment.

- `PublishVersion` / `ListVersionsByFunction`
- `CreateAlias` / `GetAlias` / `UpdateAlias` / `DeleteAlias` / `ListAliases`
- Container image deployment (`PackageType: Image`, `ImageUri`)
- Expand runtime support: all Python, Node.js, Java, .NET, Ruby, provided.al2023
- Warm container pool improvements: configurable pool size, per-version pooling
- Log capture and `X-Amz-Log-Result` header

### Phase 2: Async Invoke, Permissions, Tags, Account Settings

**Goal**: Event invocation mode, policy management, resource tagging.

- `Invoke` (Event mode) with internal async queue and background execution
- `AddPermission` / `RemovePermission` / `GetPolicy` (store policy, no enforcement)
- `TagResource` / `UntagResource` / `ListTags`
- `GetAccountSettings` (return static limits)
- Idle container cleanup background task
- Container stop/remove on function deletion

### Phase 3: Function URLs, Polish, Compatibility

**Goal**: Direct HTTP invocation via function URLs, full API compatibility.

- `CreateFunctionUrlConfig` / `GetFunctionUrlConfig` / `UpdateFunctionUrlConfig` / `DeleteFunctionUrlConfig` / `ListFunctionUrlConfigs`
- Function URL routing: generate a local URL (e.g., `http://localhost:4566/lambda-url/{function-name}/`) that forwards HTTP requests as Lambda events
- CORS support for function URLs
- Weighted alias routing (route percentage of traffic to alternate version)
- GitHub Action integration: add Lambda to `tyrchen/rustack` action
- Docker image update: mount Docker socket by default

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Docker daemon availability | High | High | Make Docker optional via `LAMBDA_DOCKER_ENABLED`. Function CRUD works without Docker; only Invoke requires it. Return clear error message when Docker is unavailable. |
| Container startup latency (cold starts) | High | Medium | Warm container pooling reduces cold starts for repeated invocations. Pre-pull images on CreateFunction. |
| Docker-in-Docker complexity | High | Medium | Recommend socket mount approach. Document host path mapping for code volumes. Provide `LAMBDA_CODE_HOST_PATH` env var. |
| RIE compatibility across runtimes | Medium | High | Test each runtime (Python, Node.js, Java) individually. The RIE is maintained by AWS and has consistent behavior. |
| Code volume mount path mismatch | Medium | High | When Rustack runs in a container, code paths inside the container differ from host paths. Implement path translation or use Docker volumes instead of bind mounts. |
| restJson1 codegen complexity | Medium | Medium | The codegen for `restJson1` is more complex than `awsJson` due to multi-location field bindings. Consider hand-writing the route table and deserialization for the initial implementation, then backfill codegen. |
| Container cleanup on abnormal exit | Medium | Medium | Register shutdown hook that stops all managed containers. Label containers with `rustack-lambda` for identification and cleanup. |
| bollard (Docker client) API stability | Low | Low | bollard is well-maintained and widely used. Pin version with `~`. |
| Memory usage with many warm containers | Medium | Medium | Default to 5 warm containers per function. Each container uses ~128MB minimum. Make configurable. |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users expect event source mappings | High | Medium | Clearly document as non-goal. Design storage to allow adding ESM later without breaking changes. |
| Users expect layers support | Medium | Medium | Accept layers parameter, store ARN list, do not download/mount. Log warning that layers are not functional. |
| SAM CLI compatibility issues | Medium | High | Test early with SAM CLI. SAM uses a subset of Lambda API (CreateFunction, Invoke, GetFunction) that the MVP covers. |
| Container image pull failures | Medium | Medium | Pre-pull on CreateFunction. Cache pulled images. Provide clear error messages for network failures. |
| S3-based code deployment expected | High | Medium | Many tools (Terraform, CDK) upload code to S3 then reference it in CreateFunction. We accept S3 params but do not fetch from S3. Workaround: use inline `ZipFile` or `ImageUri`. Document this limitation. |

### 16.3 Behavioral Differences

| Behavior | AWS Lambda | LocalStack | Rustack | Justification |
|----------|-----------|------------|-----------|---------------|
| Function state transitions | Async: Pending -> Active | Immediate Active | Immediate Active | Simpler for local dev |
| S3 code deployment | Supported | Supported (with S3) | Not supported | Avoid cross-service dependency for MVP |
| Layers | Downloaded and mounted | Supported | Metadata only | Avoid complexity |
| Concurrent execution limits | Enforced | Optional | Not enforced | Not needed for local dev |
| Cold start time | 100ms-5s | Similar | Docker-dependent | Real Docker containers |
| Log persistence | CloudWatch Logs | Stored locally | LogResult header only | Avoid CloudWatch dependency |
| Function URL format | `https://{id}.lambda-url.{region}.on.aws` | Similar | `http://localhost:4566/lambda-url/{name}/` | Local-friendly URL |
| Async invoke retry | 2 retries with backoff | Configurable | Single attempt | Simpler for local dev |

---

## Appendix A: Lambda vs Other Services Implementation Effort Comparison

| Component | DynamoDB Lines | SSM Lines | Lambda Est. | Notes |
|-----------|---------------|-----------|-------------|-------|
| Model (codegen output) | ~4,000 | ~1,500 | ~3,500 | 27 operations, moderate type complexity |
| HTTP routing | ~100 | ~100 | ~600 | restJson1 path matching is new and more complex |
| Request/response codec | ~200 | ~200 | ~800 | Multi-location binding (path+query+headers+body) |
| Auth integration | ~100 | ~100 | ~100 | SigV4 only, identical |
| Core business logic | ~6,000 | ~1,500 | ~3,000 | Function CRUD is simpler than DDB expressions |
| Storage engine | ~2,500 | ~800 | ~1,500 | DashMap + filesystem for code |
| Execution engine | N/A | N/A | ~2,500 | Docker container lifecycle (NEW) |
| Container pool | N/A | N/A | ~500 | Warm container management (NEW) |
| **Total** | **~12,900** | **~4,200** | **~12,500** | |

Lambda's total effort is comparable to DynamoDB, but the complexity is distributed differently: less in parsing/storage, more in the execution engine and restJson1 protocol handling.

## Appendix B: Lambda Error Codes and HTTP Status Codes

| Error Code | HTTP Status | When |
|-----------|------------|------|
| `InvalidParameterValueException` | 400 | Invalid request parameter |
| `InvalidRequestContentException` | 400 | Invalid JSON or header format |
| `RequestTooLargeException` | 413 | Payload exceeds quota (6MB sync, 1MB async) |
| `UnsupportedMediaTypeException` | 415 | Body content-type not JSON |
| `ResourceNotFoundException` | 404 | Function, version, or alias not found |
| `ResourceConflictException` | 409 | Resource exists or operation in progress |
| `TooManyRequestsException` | 429 | API throughput limit exceeded |
| `ServiceException` | 500 | Internal server error |
| `InvalidRuntimeException` | 502 | Unsupported runtime |
| `ResourceNotReadyException` | 502 | Function inactive or initializing |

## Appendix C: Lambda Constraints and Limits

| Resource | AWS Limit | Enforced in Rustack? |
|----------|-----------|----------------------|
| Function name length | 1-140 characters | Yes |
| Handler length | Max 128 characters | Yes |
| Description length | Max 256 characters | Yes |
| Timeout | 1-900 seconds | Yes |
| Memory | 128-10,240 MB | Yes (default 128) |
| Ephemeral storage | 512-10,240 MB | Metadata only |
| Environment variables total size | 4 KB | Yes |
| Deployment package (zip) | 50 MB (zipped), 250 MB (unzipped) | Yes |
| Synchronous payload | 6 MB | Yes |
| Asynchronous payload | 1 MB (256 KB for event invoking) | Yes |
| Versions per function | 75 | No (unbounded) |
| Aliases per function | 100 | No (unbounded) |
| Tags per function | 50 | Yes |
| Concurrent executions | 1,000 per region | No (unbounded) |
| Function URL CORS max origins | 100 | Metadata only |

## Appendix D: Supported Runtimes at Launch

| Runtime ID | Docker Image | Phase |
|-----------|-------------|-------|
| `python3.12` | `public.ecr.aws/lambda/python:3.12` | 0 |
| `nodejs20.x` | `public.ecr.aws/lambda/nodejs:20` | 0 |
| `python3.11` | `public.ecr.aws/lambda/python:3.11` | 1 |
| `python3.13` | `public.ecr.aws/lambda/python:3.13` | 1 |
| `nodejs18.x` | `public.ecr.aws/lambda/nodejs:18` | 1 |
| `nodejs22.x` | `public.ecr.aws/lambda/nodejs:22` | 1 |
| `java21` | `public.ecr.aws/lambda/java:21` | 1 |
| `java17` | `public.ecr.aws/lambda/java:17` | 1 |
| `dotnet8` | `public.ecr.aws/lambda/dotnet:8` | 1 |
| `ruby3.3` | `public.ecr.aws/lambda/ruby:3.3` | 1 |
| `ruby3.4` | `public.ecr.aws/lambda/ruby:3.4` | 1 |
| `provided.al2023` | `public.ecr.aws/lambda/provided:al2023` | 1 |
| `provided.al2` | `public.ecr.aws/lambda/provided:al2` | 1 |
| Container images | User-provided `ImageUri` | 1 |
