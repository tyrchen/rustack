# Rustack STS: Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-sns-design.md](./rustack-sns-design.md)
**Scope:** Add AWS Security Token Service (STS) support to Rustack -- 8 operations covering the core STS API surface, using the same Smithy-based codegen and gateway routing patterns established by existing services.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: awsQuery](#5-protocol-design-awsquery)
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

This spec proposes adding AWS Security Token Service (STS) support to Rustack. Key points:

- **Small scope, critical importance** -- STS has only ~8 operations total, making it one of the smallest AWS services by API surface. However, STS is arguably the most critical AWS service for SDK bootstrapping: every AWS SDK calls `GetCallerIdentity` on startup to verify credentials and determine account context. `AssumeRole` is fundamental to cross-account access patterns, CI/CD role chaining, and any application that uses IAM roles.
- **SDK bootstrapping dependency** -- without a local STS, any AWS SDK client configured against Rustack must either skip credential verification or fail on the initial `GetCallerIdentity` call. Adding STS eliminates this friction and enables transparent SDK usage against Rustack.
- **awsQuery protocol** -- STS uses the `awsQuery` protocol: `POST /` with `application/x-www-form-urlencoded` body containing `Action=OperationName`. Responses are XML. This is the same protocol family as SNS. The key routing challenge is distinguishing STS from SNS at the gateway level, which is solved by inspecting the SigV4 credential scope service name (`sts` vs `sns`).
- **Session credential management** -- STS introduces the concept of temporary security credentials (access key starting with `ASIA`, secret key, and session token). The core complexity is the credential store that maps temporary access keys back to their originating identity, and the session store that tracks assumed-role sessions with tags and transitive tag propagation for chained `AssumeRole` calls.
- **Relationship with IAM** -- STS depends on IAM conceptually (AssumeRole validates role ARNs), but must work standalone when IAM is not enabled. The design uses an optional IAM role validation interface.
- **Smithy codegen reuse** -- generate a `rustack-sts-model` crate from the official Smithy model using the same codegen infrastructure as all other services.
- **Estimated effort** -- 2-3 days for Phase 0 (4 core operations), 4-5 days for full implementation (8 operations).

---

## 2. Motivation

### 2.1 Why STS?

AWS STS is the identity foundation of the AWS ecosystem. It is called implicitly or explicitly by virtually every AWS tool:

- **SDK initialization** -- Every AWS SDK calls `GetCallerIdentity` during client construction to verify credentials and determine the caller's account ID. Without STS, SDKs fall back to error handling or skip verification, creating subtle behavioral differences.
- **AssumeRole** -- The standard pattern for cross-account access, CI/CD pipelines (GitHub Actions OIDC), and least-privilege workflows. Terraform, CDK, Pulumi, and every major IaC tool use AssumeRole.
- **Session tokens** -- Applications that obtain temporary credentials via `GetSessionToken` or `AssumeRole` need STS to validate and refresh those credentials.
- **Kubernetes IRSA** -- EKS pods use `AssumeRoleWithWebIdentity` to get AWS credentials. Local development with kind/minikube needs a local STS to test this flow.
- **CI/CD** -- GitHub Actions, GitLab CI, and CircleCI use OIDC federation (`AssumeRoleWithWebIdentity`) to obtain AWS credentials without storing long-lived secrets.

Without a local STS, developers working against Rustack must configure their SDKs to skip credential validation, which diverges from production behavior and masks configuration errors.

### 2.2 Complexity Assessment

| Dimension | STS | SNS | Secrets Manager |
|-----------|-----|-----|-----------------|
| Total operations | 8 | 19 | 23 |
| Complex state machines | 0 | 0 | 1 (rotation lifecycle) |
| Storage complexity | Credential map + Session map | Topic/Sub maps | Secret/Version/Label maps |
| Concurrency model | Request/response | Request/response + fan-out | Request/response |
| Protocol | awsQuery (reuse SNS XML infra) | awsQuery (exists) | awsJson1.1 |
| Cross-service dependency | Optional IAM role lookup | Optional SQS delivery | None |
| Estimated lines of code | ~2,000 | ~4,000 | ~4,500 |

STS is the simplest service by API surface. The core complexity lies in:
1. Temporary credential generation and lifecycle management
2. Session tag propagation across chained AssumeRole calls
3. Gateway routing disambiguation from SNS (both use awsQuery)
4. Extracting caller identity from the SigV4 Authorization header

### 2.3 Tool Coverage

With all 8 operations implemented, the following tools work out of the box:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws sts`) | GetCallerIdentity, AssumeRole, GetSessionToken | Phase 0 |
| AWS SDK (all languages) | GetCallerIdentity (bootstrap), AssumeRole | Phase 0 |
| Terraform (AWS provider) | GetCallerIdentity (init), AssumeRole (provider config) | Phase 0 |
| AWS CDK | GetCallerIdentity (bootstrap) | Phase 0 |
| GitHub Actions OIDC | AssumeRoleWithWebIdentity | Phase 1 |
| Kubernetes IRSA/EKS Pod Identity | AssumeRoleWithWebIdentity | Phase 1 |
| boto3 STS client | All operations | Phase 0 + Phase 1 |
| aws-vault | GetSessionToken, AssumeRole | Phase 0 |
| saml2aws | AssumeRoleWithSAML | Phase 1 |
| Pulumi | GetCallerIdentity (init), AssumeRole | Phase 0 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full core API** -- implement all 8 STS operations across two phases
2. **Correct GetCallerIdentity** -- return proper account ID, ARN, and user ID for both root credentials and assumed-role sessions
3. **Temporary credential generation** -- generate realistic temporary credentials with access keys starting with `ASIA`, random secret keys, and opaque session tokens
4. **Session tracking** -- maintain a session store mapping temporary access keys to the originating identity, role ARN, and session metadata
5. **Tag propagation** -- support session tags in AssumeRole, with transitive tag propagation across chained AssumeRole calls
6. **Account ID encoding** -- encode the account ID in the access key format so `GetAccessKeyInfo` can extract it without a store lookup
7. **Standalone operation** -- STS works independently when IAM is not enabled; role ARN format is validated but role existence is not required
8. **Smithy-generated types** -- all types generated from official AWS Smithy model
9. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsQuery protocol layer from SNS
10. **Same Docker image** -- single binary serves all services on port 4566

### 3.2 Non-Goals

1. **Real cryptographic token signing** -- session tokens are opaque random strings, not cryptographically signed JWT-like tokens
2. **Token expiration enforcement** -- temporary credentials do not actually expire; `DurationSeconds` is accepted and stored but not enforced
3. **IAM policy evaluation** -- AssumeRole does not evaluate the role's trust policy or permission boundaries
4. **Real SAML assertion validation** -- `AssumeRoleWithSAML` accepts any SAML response and extracts claims without cryptographic verification
5. **Real OIDC token validation** -- `AssumeRoleWithWebIdentity` accepts any JWT without signature verification against the IdP
6. **Authorization message decoding** -- `DecodeAuthorizationMessage` returns a static decoded message (no real policy encoding)
7. **Federation token policy enforcement** -- `GetFederationToken` accepts and stores policy but does not enforce it
8. **Multi-region STS endpoints** -- single global store, no regional endpoint differentiation
9. **Rate limiting** -- no request throttling
10. **Data persistence across restarts** -- in-memory only, matching all other Rustack services

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                AWS SDK / CLI / Terraform / CI/CD
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  SigV4 service=sts dispatch
              +--------+------------+
                       |
    +------+------+------+------+------+------+
    |      |      |      |      |      |
    v      v      v      v      v      v
  +---+ +-----+ +-----+ +-----+ +-----+ +---+
  |S3 | | DDB | | SQS | | SSM | | SNS | |STS|
  |Xml| |(J10)| |(Qry)| |(J11)| |(Qry)| |Qry|
  +---+ +-----+ +-----+ +-----+ +-----+ +---+
    |      |       |       |       |       |
  +---+ +-----+ +-----+ +-----+ +-----+ +---+
  |S3 | |DDB  | |SQS  | |SSM  | |SNS  | |STS|
  |Cor| |Core | |Core | |Core | |Core | |Cor|
  +---+ +-----+ +-----+ +-----+ +-----+ +---+
    |      |       |       |       |       |
    +------+-------+-------+-------+-------+
                       |
                +------+------+
                | rustack-  |
                | core + auth |
                +-------------+
```

### 4.2 Gateway Routing

STS uses the `awsQuery` protocol, which is the same wire format as SNS. Both services accept `POST /` with `Content-Type: application/x-www-form-urlencoded` and respond with `text/xml`. This creates a routing challenge: the gateway cannot distinguish STS from SNS by Content-Type or X-Amz-Target header (neither uses X-Amz-Target).

**Routing strategy: SigV4 credential scope service name.**

Every authenticated AWS request includes an `Authorization` header with a credential scope that contains the signing service name:

```
Authorization: AWS4-HMAC-SHA256
  Credential=AKIAIOSFODNN7EXAMPLE/20260319/us-east-1/sts/aws4_request,
  SignedHeaders=content-type;host;x-amz-date,
  Signature=...
```

The credential scope contains `sts` for STS requests and `sns` for SNS requests. The gateway parses the `Authorization` header to extract the service name and routes accordingly.

| Service | Protocol | Routing Mechanism |
|---------|----------|-------------------|
| DynamoDB | awsJson1.0 | `X-Amz-Target: DynamoDB_20120810.*` |
| SSM | awsJson1.1 | `X-Amz-Target: AmazonSSM.*` |
| Secrets Manager | awsJson1.1 | `X-Amz-Target: secretsmanager.*` |
| KMS | awsJson1.1 | `X-Amz-Target: TrentService.*` |
| EventBridge | awsJson1.1 | `X-Amz-Target: AWSEvents.*` |
| CloudWatch Logs | awsJson1.1 | `X-Amz-Target: Logs_20140328.*` |
| Kinesis | awsJson1.1 | `X-Amz-Target: Kinesis_20131202.*` |
| SQS | awsQuery/JSON | `X-Amz-Target: AmazonSQS.*` |
| SNS | awsQuery | `Content-Type: x-www-form-urlencoded` + SigV4 `service=sns` (fallback) |
| STS | awsQuery | `Content-Type: x-www-form-urlencoded` + SigV4 `service=sts` |
| Lambda | restJson1 | URL path pattern `/YYYY-MM-DD/functions` |
| S3 | restXml | Catch-all (last in router order) |

**Router ordering update:** STS must be registered before SNS in the gateway's router list. The STS router checks for `Content-Type: x-www-form-urlencoded` AND SigV4 `service=sts`. If both match, STS handles the request. SNS remains the fallback for form-urlencoded POST requests with `service=sns` or no service match.

Alternatively, SNS can be updated to also check `service=sns` in the credential scope, making both services explicit rather than SNS being a catch-all for form-urlencoded. This is the preferred approach for clarity and correctness.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-sns-{model,core,http}
+-- rustack-sts-model                    <-- NEW (auto-generated)
+-- rustack-sts-core                     <-- NEW
+-- rustack-sts-http                     <-- NEW
+-- ...other services...

rustack-sts-http
+-- rustack-sts-model
+-- rustack-auth                         (for parsing SigV4 Authorization header)

rustack-sts-core
+-- rustack-core
+-- rustack-sts-model

rustack-sts-model (auto-generated, standalone)
```

---

## 5. Protocol Design: awsQuery

### 5.1 Protocol Comparison

STS uses `awsQuery`, which is shared with SNS. The wire format is identical except for the XML namespace, operation names, and SigV4 service name.

| Aspect | SNS (awsQuery) | STS (awsQuery) |
|--------|----------------|----------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type (request) | `application/x-www-form-urlencoded` | `application/x-www-form-urlencoded` |
| Content-Type (response) | `text/xml` | `text/xml` |
| Operation routing | `Action=` form parameter | `Action=` form parameter |
| X-Amz-Target | *(absent)* | *(absent)* |
| Request body | URL-encoded key-value pairs | URL-encoded key-value pairs |
| Response body | XML | XML |
| Error format | XML `<ErrorResponse>` | XML `<ErrorResponse>` |
| XML namespace | `http://sns.amazonaws.com/doc/2010-03-31/` | `https://sts.amazonaws.com/doc/2011-06-15/` |
| Auth | SigV4, `service=sns` | SigV4, `service=sts` |

### 5.2 What We Reuse from SNS

The SNS implementation provides the core awsQuery infrastructure that STS can reuse:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| Form parameter parsing (`parse_form_params`) | Yes | Same URL-encoded parsing |
| XML response formatting (`XmlWriter`) | Yes | Same XML builder, different namespace |
| XML error formatting | Yes | Same `<ErrorResponse>` structure |
| `xml_escape` utility | Yes | Identical |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| `Action=` parameter routing | Yes | Same pattern |

The shared XML infrastructure should be extracted into a common crate or module if not already shared. Since both SNS and STS use the same `XmlWriter` pattern, the STS HTTP crate can either depend on `rustack-sns-http` for the utilities or duplicate the small amount of code. **Preferred approach:** extract the `XmlWriter`, `xml_escape`, and awsQuery error formatting into `rustack-core` or a new `rustack-query-protocol` utility module, then both SNS and STS depend on it. For the initial implementation, STS can duplicate the ~100 lines of XML utilities to avoid a cross-service dependency.

### 5.3 Wire Format Examples

**GetCallerIdentity request:**

```
POST / HTTP/1.1
Host: sts.us-east-1.amazonaws.com
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260319/us-east-1/sts/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=...
X-Amz-Date: 20260319T120000Z

Action=GetCallerIdentity&Version=2011-06-15
```

**GetCallerIdentity response:**

```xml
<GetCallerIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <GetCallerIdentityResult>
    <Arn>arn:aws:iam::000000000000:root</Arn>
    <UserId>000000000000</UserId>
    <Account>000000000000</Account>
  </GetCallerIdentityResult>
  <ResponseMetadata>
    <RequestId>01234567-89ab-cdef-0123-456789abcdef</RequestId>
  </ResponseMetadata>
</GetCallerIdentityResponse>
```

**AssumeRole request:**

```
POST / HTTP/1.1
Host: sts.us-east-1.amazonaws.com
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20260319/us-east-1/sts/aws4_request, ...

Action=AssumeRole&Version=2011-06-15&RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2FMyRole&RoleSessionName=my-session&DurationSeconds=3600&Tags.member.1.Key=Project&Tags.member.1.Value=MyProject
```

**AssumeRole response:**

```xml
<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <Credentials>
      <AccessKeyId>ASIAQWERTYUIOPASDFGH</AccessKeyId>
      <SecretAccessKey>wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY</SecretAccessKey>
      <SessionToken>FwoGZXIvYXdzEBYaDH...long-opaque-token...</SessionToken>
      <Expiration>2026-03-19T13:00:00Z</Expiration>
    </Credentials>
    <AssumedRoleUser>
      <AssumedRoleId>AROAQWERTYUIOPASDFGH:my-session</AssumedRoleId>
      <Arn>arn:aws:sts::123456789012:assumed-role/MyRole/my-session</Arn>
    </AssumedRoleUser>
    <PackedPolicySize>6</PackedPolicySize>
  </AssumeRoleResult>
  <ResponseMetadata>
    <RequestId>01234567-89ab-cdef-0123-456789abcdef</RequestId>
  </ResponseMetadata>
</AssumeRoleResponse>
```

**Error response:**

```xml
<ErrorResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <Error>
    <Type>Sender</Type>
    <Code>MalformedPolicyDocument</Code>
    <Message>The policy document is malformed.</Message>
  </Error>
  <RequestId>01234567-89ab-cdef-0123-456789abcdef</RequestId>
</ErrorResponse>
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-sts-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The STS Smithy model must first be downloaded and placed at `codegen/smithy-model/sts.json`.

**Smithy model:** `codegen/smithy-model/sts.json` (namespace `com.amazonaws.sts`, 8 operations)
**Service config:** `codegen/services/sts.toml`
**Generate:** `make codegen-sts`

### 6.2 Service Configuration (codegen/services/sts.toml)

```toml
[service]
name = "sts"
display_name = "STS"
rust_prefix = "Sts"
namespace = "com.amazonaws.sts"
protocol = "awsQuery"

[protocol]
serde_rename = "PascalCase"
emit_serde_derives = true

[operations]
phase0 = [
    "GetCallerIdentity", "AssumeRole",
    "GetSessionToken", "GetAccessKeyInfo",
]
phase1 = [
    "AssumeRoleWithSAML", "AssumeRoleWithWebIdentity",
    "DecodeAuthorizationMessage", "GetFederationToken",
]

[errors.custom]
MissingAction = { status = 400, message = "Missing action parameter" }
InvalidAction = { status = 400, message = "Operation is not supported" }

[output]
file_layout = "flat"
```

### 6.3 Generated Output

The codegen produces 6 files in `crates/rustack-sts-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (Credentials, AssumedRoleUser, Tag, PolicyDescriptorType, etc.) |
| `operations.rs` | `StsOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `StsErrorCode` enum + `StsError` struct + `sts_error!` macro |
| `input.rs` | All 8 input structs with serde derives |
| `output.rs` | All 8 output structs with serde derives |

### 6.4 Service-Specific Notes

STS uses the `awsQuery` protocol, which means request parameters are URL-encoded form fields rather than JSON. The generated model types will use serde derives for potential JSON usage in tests, but the HTTP layer will parse form parameters manually (like SNS) and construct model types from the parsed key-value pairs. The XML response serialization is also manual (using `XmlWriter`), not serde-driven.

The awsQuery protocol uses a specific convention for nested types in form parameters:
- Flat lists: `Tags.member.1.Key=Foo&Tags.member.1.Value=Bar`
- Simple fields: `RoleArn=arn%3A...&RoleSessionName=my-session`

The form parameter parser must handle this `member.N.Field` convention for tags and policy descriptors.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-sts-model` (auto-generated)

```
crates/rustack-sts-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: Credentials, AssumedRoleUser, Tag, etc.
    +-- operations.rs       # Auto-generated: StsOperation enum
    +-- error.rs            # Auto-generated: error types + error codes
    +-- input.rs            # Auto-generated: all 8 input structs
    +-- output.rs           # Auto-generated: all 8 output structs
```

**Dependencies:** `serde`, `serde_json`

### 7.2 `rustack-sts-core`

```
crates/rustack-sts-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # StsConfig
    +-- handler.rs          # StsHandler trait (all 8 operation dispatch)
    +-- provider.rs         # RustackSts (main provider, all operation handlers)
    +-- credential.rs       # CredentialStore: maps access key IDs to identity info
    +-- session.rs          # SessionStore: tracks assumed-role sessions with tags
    +-- identity.rs         # CallerIdentity resolution from access key
    +-- keygen.rs           # Temporary credential generation (ASIA keys, session tokens)
    +-- validation.rs       # Role ARN, session name, tag validation
```

**Dependencies:** `rustack-core`, `rustack-sts-model`, `dashmap`, `tracing`, `rand`, `uuid`, `base64`, `chrono`

### 7.3 `rustack-sts-http`

```
crates/rustack-sts-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # Action= form parameter dispatch
    +-- service.rs          # StsHttpService (hyper Service impl)
    +-- dispatch.rs         # StsHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- response.rs         # XML response construction, XmlWriter
    +-- request.rs          # Form parameter parsing (reuse or duplicate from SNS)
```

**Dependencies:** `rustack-sts-model`, `rustack-auth`, `hyper`, `http`, `bytes`, `uuid`

This crate is structurally identical to `rustack-sns-http`. The router parses `Action=<Op>` from form parameters.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-sts-model = { path = "crates/rustack-sts-model" }
rustack-sts-http = { path = "crates/rustack-sts-http" }
rustack-sts-core = { path = "crates/rustack-sts-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// STS operation router.
///
/// Parses the `Action` form parameter to determine the STS operation.
pub fn resolve_operation(params: &[(String, String)]) -> Result<StsOperation, StsError> {
    let action = params
        .iter()
        .find(|(k, _)| k == "Action")
        .map(|(_, v)| v.as_str())
        .ok_or_else(StsError::missing_action)?;

    StsOperation::from_name(action).ok_or_else(|| StsError::unknown_operation(action))
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// STS service router for the gateway.
///
/// Matches form-urlencoded POST requests where the SigV4 credential scope
/// service name is `sts`.
pub struct StsServiceRouter<H: StsHandler> {
    inner: StsHttpService<H>,
}

impl<H: StsHandler> ServiceRouter for StsServiceRouter<H> {
    fn name(&self) -> &'static str {
        "sts"
    }

    /// STS matches form-urlencoded POST requests signed with service=sts.
    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        if *req.method() != http::Method::POST {
            return false;
        }

        let is_form_encoded = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("x-www-form-urlencoded"));

        if !is_form_encoded {
            return false;
        }

        // Check SigV4 credential scope for service=sts
        req.headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|auth| extract_sigv4_service(auth) == Some("sts"))
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

/// Extract the service name from a SigV4 Authorization header.
///
/// Parses the Credential component to find the service in the credential scope:
/// `Credential=AKID/YYYYMMDD/region/SERVICE/aws4_request`
fn extract_sigv4_service(auth_header: &str) -> Option<&str> {
    // Find "Credential=" and extract the credential scope
    let cred_start = auth_header.find("Credential=")?;
    let cred_value = &auth_header[cred_start + 11..];
    let cred_end = cred_value.find([',', ' ']).unwrap_or(cred_value.len());
    let credential = &cred_value[..cred_end];

    // Credential format: AKID/YYYYMMDD/region/service/aws4_request
    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() >= 4 {
        Some(parts[3])
    } else {
        None
    }
}
```

### 8.3 Handler Trait

```rust
/// Trait that the STS business logic provider must implement.
///
/// The HTTP layer parses form parameters and the Authorization header,
/// then calls the appropriate handler method.
pub trait StsHandler: Send + Sync + 'static {
    /// Handle an STS operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: StsOperation,
        params: Vec<(String, String)>,
        caller_access_key: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<
        http::Response<StsResponseBody>,
        StsError,
    >> + Send>>;
}
```

Note the `caller_access_key` parameter: the STS handler needs the access key ID from the request's Authorization header to determine caller identity for `GetCallerIdentity`. This is extracted by the HTTP layer from the SigV4 credential scope.

### 8.4 Request Processing Pipeline

```rust
/// Process a single STS HTTP request through the full pipeline.
///
/// Pipeline:
/// 1. Verify POST method
/// 2. Collect body
/// 3. Parse form params from body
/// 4. Resolve operation from `Action=` param
/// 5. Extract caller access key from Authorization header
/// 6. Authenticate (if enabled)
/// 7. Dispatch to handler (pass parsed params + caller identity)
async fn process_request<H: StsHandler>(
    req: http::Request<Incoming>,
    handler: &H,
    config: &StsHttpConfig,
    request_id: &str,
) -> http::Response<StsResponseBody> {
    let (parts, incoming) = req.into_parts();

    // 1. Verify POST method.
    if parts.method != http::Method::POST {
        let err = StsError::invalid_action("STS requires POST method");
        return error_to_response(&err, request_id);
    }

    // 2. Collect body.
    let body = match collect_body(incoming).await {
        Ok(body) => body,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 3. Parse form params.
    let params = parse_form_params(&body);

    // 4. Resolve operation.
    let op = match resolve_operation(&params) {
        Ok(op) => op,
        Err(err) => return error_to_response(&err, request_id),
    };

    // 5. Extract caller access key from Authorization header.
    let caller_access_key = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(extract_access_key_from_auth);

    // 6. Authenticate (if enabled).
    if !config.skip_signature_validation {
        if let Some(ref cred_provider) = config.credential_provider {
            let body_hash = rustack_auth::hash_payload(&body);
            if let Err(auth_err) =
                rustack_auth::verify_sigv4(&parts, &body_hash, cred_provider.as_ref())
            {
                let err = StsError::invalid_client_token_id(auth_err.to_string());
                return error_to_response(&err, request_id);
            }
        }
    }

    // 7. Dispatch to handler.
    match handler.handle_operation(op, params, caller_access_key).await {
        Ok(response) => response,
        Err(err) => error_to_response(&err, request_id),
    }
}

/// Extract the access key ID from a SigV4 Authorization header.
fn extract_access_key_from_auth(auth_header: &str) -> Option<String> {
    let cred_start = auth_header.find("Credential=")?;
    let cred_value = &auth_header[cred_start + 11..];
    let cred_end = cred_value.find('/')?;
    Some(cred_value[..cred_end].to_owned())
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The STS storage model centers on two concepts:

1. **Credential store** -- maps access key IDs (both permanent `AKIA*` and temporary `ASIA*`) to their associated identity (account, role, session info). This is how `GetCallerIdentity` resolves who is calling.
2. **Session store** -- tracks active assumed-role sessions with their tags, transitive tags, and metadata. This enables tag propagation across chained `AssumeRole` calls.

### 9.2 Core Data Structures

```rust
/// Top-level STS state.
///
/// Global singleton (STS is a global service). Not scoped by region.
pub struct StsState {
    /// Maps access key ID -> caller identity info.
    /// Includes both permanent credentials (AKIA*) and temporary credentials (ASIA*).
    pub credentials: DashMap<String, CredentialRecord>,

    /// Maps session token -> session record.
    /// Used for looking up session details (tags, source identity) when
    /// temporary credentials are used in subsequent requests.
    pub sessions: DashMap<String, SessionRecord>,

    /// Default account ID for root credentials.
    pub default_account_id: String,

    /// Default access key ID that maps to the root account.
    /// This is the access key used by default SDK configuration.
    pub default_access_key: String,
}

/// A record associating an access key with its identity.
#[derive(Debug, Clone)]
pub struct CredentialRecord {
    /// The access key ID (AKIA* for permanent, ASIA* for temporary).
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: String,
    /// The session token (only for temporary credentials).
    pub session_token: Option<String>,
    /// The identity associated with this credential.
    pub identity: CallerIdentity,
    /// When these credentials expire (epoch seconds).
    /// Not enforced, but returned in responses.
    pub expiration: Option<f64>,
}

/// The identity of a caller, resolved from their access key.
#[derive(Debug, Clone)]
pub enum CallerIdentity {
    /// Root account credentials.
    Root {
        /// Account ID (e.g., "000000000000").
        account_id: String,
    },
    /// IAM user credentials.
    User {
        /// Account ID.
        account_id: String,
        /// User name.
        user_name: String,
        /// User ID (unique identifier, e.g., "AIDAQWERTYUIOPASDFGHJ").
        user_id: String,
    },
    /// Assumed role session.
    AssumedRole {
        /// Account ID.
        account_id: String,
        /// Role name (extracted from role ARN).
        role_name: String,
        /// Session name (provided by caller in AssumeRole).
        session_name: String,
        /// Role session ID (unique identifier, e.g., "AROAQWERTYUIOPASDFGHJ").
        role_id: String,
        /// Session token reference for tag lookup.
        session_token: String,
    },
    /// Federated user session.
    FederatedUser {
        /// Account ID.
        account_id: String,
        /// Federated user name.
        federated_user_name: String,
        /// Federated user ID.
        federated_user_id: String,
    },
}

impl CallerIdentity {
    /// Return the account ID for this identity.
    pub fn account_id(&self) -> &str {
        match self {
            Self::Root { account_id }
            | Self::User { account_id, .. }
            | Self::AssumedRole { account_id, .. }
            | Self::FederatedUser { account_id, .. } => account_id,
        }
    }

    /// Return the ARN for this identity.
    pub fn arn(&self) -> String {
        match self {
            Self::Root { account_id } => {
                format!("arn:aws:iam::{account_id}:root")
            }
            Self::User {
                account_id,
                user_name,
                ..
            } => {
                format!("arn:aws:iam::{account_id}:user/{user_name}")
            }
            Self::AssumedRole {
                account_id,
                role_name,
                session_name,
                ..
            } => {
                format!(
                    "arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}"
                )
            }
            Self::FederatedUser {
                account_id,
                federated_user_name,
                ..
            } => {
                format!(
                    "arn:aws:sts::{account_id}:federated-user/{federated_user_name}"
                )
            }
        }
    }

    /// Return the user ID for this identity.
    ///
    /// For root: the account ID.
    /// For IAM users: the user ID.
    /// For assumed roles: `ROLE_ID:SESSION_NAME`.
    /// For federated users: `ACCOUNT_ID:FEDERATED_USER_NAME`.
    pub fn user_id(&self) -> String {
        match self {
            Self::Root { account_id } => account_id.clone(),
            Self::User { user_id, .. } => user_id.clone(),
            Self::AssumedRole {
                role_id,
                session_name,
                ..
            } => {
                format!("{role_id}:{session_name}")
            }
            Self::FederatedUser {
                account_id,
                federated_user_name,
                ..
            } => {
                format!("{account_id}:{federated_user_name}")
            }
        }
    }
}

/// A session created by AssumeRole, AssumeRoleWithSAML, or AssumeRoleWithWebIdentity.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// The role ARN that was assumed.
    pub role_arn: String,
    /// The session name provided by the caller.
    pub session_name: String,
    /// Session tags provided by the caller.
    pub tags: Vec<SessionTag>,
    /// Transitive tag keys: these tags propagate to chained AssumeRole calls.
    pub transitive_tag_keys: Vec<String>,
    /// Tags inherited from the parent session (for chained AssumeRole).
    pub inherited_transitive_tags: Vec<SessionTag>,
    /// The access key ID of the temporary credentials for this session.
    pub access_key_id: String,
    /// Source identity (if provided).
    pub source_identity: Option<String>,
    /// When this session was created (epoch seconds).
    pub created_at: f64,
    /// Duration in seconds (not enforced).
    pub duration_seconds: i32,
    /// External ID (if provided in AssumeRole).
    pub external_id: Option<String>,
    /// Policy ARNs attached to the session.
    pub policy_arns: Vec<String>,
    /// Inline policy JSON.
    pub policy: Option<String>,
}

/// A session tag (key-value pair).
#[derive(Debug, Clone)]
pub struct SessionTag {
    /// Tag key.
    pub key: String,
    /// Tag value.
    pub value: String,
}
```

### 9.3 Temporary Credential Generation

AWS uses specific prefixes for access keys to distinguish their type:

| Prefix | Type | Example |
|--------|------|---------|
| `AKIA` | Permanent (IAM user) | `AKIAIOSFODNN7EXAMPLE` |
| `ASIA` | Temporary (STS) | `ASIAQWERTYUIOPASDFGH` |
| `AROA` | Role ID | `AROAQWERTYUIOPASDFGH` |

The access key format also encodes the account ID. AWS access keys are 20 characters: 4-character prefix + 16-character encoded value. For simplicity in local dev, we encode the account ID in a deterministic but opaque way.

```rust
/// Generate temporary AWS credentials.
///
/// Produces:
/// - Access key ID starting with "ASIA" (20 chars total)
/// - Random secret access key (40 chars, base64-like)
/// - Opaque session token (random, 356 chars to match AWS format)
pub struct CredentialGenerator {
    /// Account ID to encode in the access key.
    account_id: String,
}

impl CredentialGenerator {
    /// Create a new credential generator for the given account.
    pub fn new(account_id: String) -> Self {
        Self { account_id }
    }

    /// Generate a set of temporary credentials.
    pub fn generate_temporary(&self) -> GeneratedCredentials {
        let access_key_id = self.generate_access_key("ASIA");
        let secret_access_key = self.generate_secret_key();
        let session_token = self.generate_session_token();

        GeneratedCredentials {
            access_key_id,
            secret_access_key,
            session_token,
        }
    }

    /// Generate a permanent-style access key (for default root).
    pub fn generate_permanent(&self) -> (String, String) {
        let access_key_id = self.generate_access_key("AKIA");
        let secret_access_key = self.generate_secret_key();
        (access_key_id, secret_access_key)
    }

    /// Generate an access key ID with the given prefix.
    ///
    /// Format: PREFIX (4 chars) + encoded account ID (4 chars) + random (12 chars)
    /// Total: 20 characters, all uppercase alphanumeric.
    fn generate_access_key(&self, prefix: &str) -> String {
        use rand::Rng;
        let mut rng = rand::rng();

        // Encode account ID into 4 base-36 characters
        let account_num: u64 = self.account_id.parse().unwrap_or(0);
        let account_encoded = encode_base36(account_num, 4);

        // Generate 12 random uppercase alphanumeric characters
        let random_part: String = (0..12)
            .map(|_| {
                let idx = rng.random_range(0..36);
                if idx < 10 {
                    (b'0' + idx as u8) as char
                } else {
                    (b'A' + (idx - 10) as u8) as char
                }
            })
            .collect();

        format!("{prefix}{account_encoded}{random_part}")
    }

    /// Generate a 40-character secret access key.
    fn generate_secret_key(&self) -> String {
        use rand::Rng;
        let mut rng = rand::rng();
        let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        (0..40)
            .map(|_| {
                let idx = rng.random_range(0..charset.len());
                charset[idx] as char
            })
            .collect()
    }

    /// Generate an opaque session token.
    ///
    /// Real AWS session tokens are ~356 characters. We generate a shorter
    /// but still opaque token for local development.
    fn generate_session_token(&self) -> String {
        use rand::Rng;
        let mut rng = rand::rng();
        let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";
        let prefix = "FwoGZXIvYXdzE"; // Mimics real AWS token prefix
        let random_part: String = (0..200)
            .map(|_| {
                let idx = rng.random_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        format!("{prefix}{random_part}")
    }
}

/// Generated temporary credential set.
#[derive(Debug, Clone)]
pub struct GeneratedCredentials {
    /// Access key ID (starts with "ASIA", 20 chars).
    pub access_key_id: String,
    /// Secret access key (40 chars).
    pub secret_access_key: String,
    /// Session token (opaque, ~213 chars).
    pub session_token: String,
}

/// Encode a number in base-36 with fixed width (uppercase).
fn encode_base36(mut num: u64, width: usize) -> String {
    let chars = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut result = vec![b'0'; width];
    for i in (0..width).rev() {
        result[i] = chars[(num % 36) as usize];
        num /= 36;
    }
    String::from_utf8(result).expect("base36 chars are valid UTF-8")
}

/// Extract the account ID from an access key.
///
/// Decodes the 4-character account ID segment (characters 4-7) from
/// base-36 back to a numeric account ID.
pub fn account_id_from_access_key(access_key: &str) -> Option<String> {
    if access_key.len() < 8 {
        return None;
    }
    let encoded = &access_key[4..8];
    let num = decode_base36(encoded)?;
    Some(format!("{num:012}"))
}

/// Decode a base-36 string to a number.
fn decode_base36(s: &str) -> Option<u64> {
    let mut result: u64 = 0;
    for ch in s.chars() {
        let digit = match ch {
            '0'..='9' => (ch as u64) - ('0' as u64),
            'A'..='Z' => (ch as u64) - ('A' as u64) + 10,
            'a'..='z' => (ch as u64) - ('a' as u64) + 10,
            _ => return None,
        };
        result = result.checked_mul(36)?.checked_add(digit)?;
    }
    Some(result)
}
```

### 9.4 Default Root Credentials

On startup, STS registers default root credentials that map the default access key to the root account:

```rust
impl StsState {
    /// Create a new STS state with default root credentials.
    pub fn new(config: &StsConfig) -> Self {
        let state = Self {
            credentials: DashMap::new(),
            sessions: DashMap::new(),
            default_account_id: config.default_account_id.clone(),
            default_access_key: config.default_access_key.clone(),
        };

        // Register default root credentials.
        // Any access key not found in the credential store is also
        // treated as root for the default account (permissive mode).
        state.credentials.insert(
            config.default_access_key.clone(),
            CredentialRecord {
                access_key_id: config.default_access_key.clone(),
                secret_access_key: config.default_secret_key.clone(),
                session_token: None,
                identity: CallerIdentity::Root {
                    account_id: config.default_account_id.clone(),
                },
                expiration: None,
            },
        );

        state
    }

    /// Resolve the caller identity from an access key ID.
    ///
    /// If the access key is found in the credential store, returns
    /// the stored identity. If not found, returns root identity for
    /// the default account (permissive mode for local development).
    pub fn resolve_identity(&self, access_key_id: &str) -> CallerIdentity {
        if let Some(record) = self.credentials.get(access_key_id) {
            return record.identity.clone();
        }

        // Permissive fallback: unknown access keys are treated as root.
        // This matches LocalStack behavior where any access key works.
        CallerIdentity::Root {
            account_id: self.default_account_id.clone(),
        }
    }
}
```

### 9.5 Tag Propagation

When `AssumeRole` is called with session tags, and the resulting temporary credentials are used to call `AssumeRole` again (chained role assumption), transitive tags from the original session must propagate to the new session.

```rust
impl StsState {
    /// Resolve the effective tags for a new AssumeRole call.
    ///
    /// Combines:
    /// 1. Tags explicitly provided in the AssumeRole request
    /// 2. Transitive tags inherited from the caller's session
    ///    (only if the caller is using temporary credentials from a prior AssumeRole)
    ///
    /// Explicit tags override inherited transitive tags with the same key.
    pub fn resolve_session_tags(
        &self,
        caller_access_key: &str,
        request_tags: &[SessionTag],
        request_transitive_keys: &[String],
    ) -> (Vec<SessionTag>, Vec<String>) {
        let mut effective_tags: Vec<SessionTag> = Vec::new();
        let mut effective_transitive_keys: Vec<String> = request_transitive_keys.to_vec();

        // Check if the caller has an existing session with transitive tags.
        if let Some(cred_record) = self.credentials.get(caller_access_key) {
            if let Some(ref token) = cred_record.session_token {
                if let Some(session) = self.sessions.get(token) {
                    // Inherit transitive tags from parent session.
                    for inherited_tag in &session.inherited_transitive_tags {
                        // Only include if the key is marked as transitive
                        // in the parent session.
                        effective_tags.push(inherited_tag.clone());
                    }
                    for tag in &session.tags {
                        if session.transitive_tag_keys.contains(&tag.key) {
                            effective_tags.push(tag.clone());
                            if !effective_transitive_keys.contains(&tag.key) {
                                effective_transitive_keys.push(tag.key.clone());
                            }
                        }
                    }
                }
            }
        }

        // Request tags override inherited tags.
        let request_tag_keys: Vec<&str> =
            request_tags.iter().map(|t| t.key.as_str()).collect();
        effective_tags.retain(|t| !request_tag_keys.contains(&t.key.as_str()));
        effective_tags.extend(request_tags.iter().cloned());

        (effective_tags, effective_transitive_keys)
    }
}
```

### 9.6 Concurrency Model

Like other Rustack services, STS uses `DashMap` for concurrent access. The workload is entirely request/response with no background processing, streaming, or real-time constraints.

- **Reads** (GetCallerIdentity, GetAccessKeyInfo): lock-free concurrent reads via DashMap
- **Writes** (AssumeRole, GetSessionToken): per-entry write locks via DashMap

No background processing is needed since temporary credentials do not actually expire.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main STS provider implementing all operations.
pub struct RustackSts {
    pub(crate) state: Arc<StsState>,
    pub(crate) config: Arc<StsConfig>,
}

impl RustackSts {
    /// Create a new STS provider.
    pub fn new(config: StsConfig) -> Self {
        let state = StsState::new(&config);
        Self {
            state: Arc::new(state),
            config: Arc::new(config),
        }
    }
}
```

### 10.2 Operations

#### Phase 0: Core (4 operations)

**GetCallerIdentity** -- Returns the account, ARN, and user ID for the caller.

This is the most-called STS operation. Every AWS SDK calls it on startup.

Algorithm:
1. Extract the caller's access key ID from the request's `Authorization` header (passed by the HTTP layer).
2. Look up the access key in the credential store.
3. If found, return the stored identity (account, ARN, user ID).
4. If not found, return root identity for the default account (permissive mode).
5. For root: ARN is `arn:aws:iam::{ACCOUNT}:root`, user ID is the account ID.
6. For assumed roles: ARN is `arn:aws:sts::{ACCOUNT}:assumed-role/{ROLE}/{SESSION}`, user ID is `{ROLE_ID}:{SESSION}`.

```rust
/// Handle GetCallerIdentity.
///
/// Returns the account ID, ARN, and user ID for the entity whose
/// credentials were used to sign the request.
pub fn get_caller_identity(
    &self,
    caller_access_key: Option<&str>,
) -> GetCallerIdentityOutput {
    let identity = match caller_access_key {
        Some(key) => self.state.resolve_identity(key),
        None => CallerIdentity::Root {
            account_id: self.config.default_account_id.clone(),
        },
    };

    GetCallerIdentityOutput {
        account: identity.account_id().to_owned(),
        arn: identity.arn(),
        user_id: identity.user_id(),
    }
}
```

**AssumeRole** -- Returns temporary credentials for an assumed role.

This is the most complex STS operation. It involves credential generation, session creation, tag handling, and validation.

Algorithm:
1. Validate `RoleArn` format: must match `arn:aws:iam::\d{12}:role/.+`.
2. Validate `RoleSessionName`: must match `[a-zA-Z_0-9+=,.@-]{2,64}`.
3. Extract account ID and role name from the role ARN.
4. *(Optional)* If IAM is enabled, validate the role exists.
5. Parse session tags from form parameters (`Tags.member.N.Key`, `Tags.member.N.Value`).
6. Parse transitive tag keys from form parameters (`TransitiveTagKeys.member.N`).
7. Resolve effective tags (merge caller's inherited transitive tags with request tags).
8. Generate temporary credentials: access key (ASIA*), secret key, session token.
9. Generate a role ID (AROA* prefix) for the assumed role.
10. Create `CredentialRecord` with `CallerIdentity::AssumedRole`.
11. Create `SessionRecord` with tags, transitive tags, external ID, policy, etc.
12. Store credential and session records.
13. Compute expiration timestamp (now + DurationSeconds, default 3600).
14. Return `Credentials` + `AssumedRoleUser`.

```rust
/// Handle AssumeRole.
pub fn assume_role(
    &self,
    caller_access_key: Option<&str>,
    params: &[(String, String)],
) -> Result<AssumeRoleOutput, StsError> {
    // 1. Extract and validate RoleArn.
    let role_arn = get_required_param(params, "RoleArn")?;
    validate_role_arn(&role_arn)?;

    // 2. Extract and validate RoleSessionName.
    let session_name = get_required_param(params, "RoleSessionName")?;
    validate_session_name(&session_name)?;

    // 3. Parse account ID and role name from ARN.
    let (account_id, role_name) = parse_role_arn(&role_arn)?;

    // 4. Parse optional parameters.
    let duration_seconds: i32 = get_optional_param(params, "DurationSeconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);
    let external_id = get_optional_param(params, "ExternalId");
    let policy = get_optional_param(params, "Policy");
    let source_identity = get_optional_param(params, "SourceIdentity");

    // 5. Parse session tags.
    let tags = parse_session_tags(params);
    let transitive_keys = parse_transitive_tag_keys(params);

    // 6. Resolve effective tags (merge with inherited transitive tags).
    let (effective_tags, effective_transitive_keys) = self.state.resolve_session_tags(
        caller_access_key.unwrap_or(""),
        &tags,
        &transitive_keys,
    );

    // 7. Generate temporary credentials.
    let gen = CredentialGenerator::new(account_id.clone());
    let creds = gen.generate_temporary();
    let role_id = generate_role_id();

    let now = current_epoch_seconds();
    let expiration = now + f64::from(duration_seconds);

    // 8. Store credential record.
    self.state.credentials.insert(
        creds.access_key_id.clone(),
        CredentialRecord {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: Some(creds.session_token.clone()),
            identity: CallerIdentity::AssumedRole {
                account_id: account_id.clone(),
                role_name: role_name.clone(),
                session_name: session_name.clone(),
                role_id: role_id.clone(),
                session_token: creds.session_token.clone(),
            },
            expiration: Some(expiration),
        },
    );

    // 9. Store session record.
    self.state.sessions.insert(
        creds.session_token.clone(),
        SessionRecord {
            role_arn: role_arn.clone(),
            session_name: session_name.clone(),
            tags: effective_tags.clone(),
            transitive_tag_keys: effective_transitive_keys,
            inherited_transitive_tags: Vec::new(), // already merged
            access_key_id: creds.access_key_id.clone(),
            source_identity,
            created_at: now,
            duration_seconds,
            external_id,
            policy_arns: parse_policy_arns(params),
            policy,
        },
    );

    // 10. Build response.
    Ok(AssumeRoleOutput {
        credentials: Credentials {
            access_key_id: creds.access_key_id,
            secret_access_key: creds.secret_access_key,
            session_token: creds.session_token,
            expiration: format_iso8601(expiration),
        },
        assumed_role_user: AssumedRoleUser {
            assumed_role_id: format!("{role_id}:{session_name}"),
            arn: format!(
                "arn:aws:sts::{account_id}:assumed-role/{role_name}/{session_name}"
            ),
        },
        packed_policy_size: Some(6), // Stub: minimal policy size percentage
        source_identity: None,
    })
}
```

**GetSessionToken** -- Returns temporary credentials for the current caller.

Algorithm:
1. Extract caller identity from access key.
2. Generate temporary credentials.
3. Store credential record with the same identity as the caller.
4. Return credentials.

```rust
/// Handle GetSessionToken.
///
/// Similar to AssumeRole but does not change the caller's identity.
/// The temporary credentials have the same permissions as the caller.
pub fn get_session_token(
    &self,
    caller_access_key: Option<&str>,
    params: &[(String, String)],
) -> Result<GetSessionTokenOutput, StsError> {
    let identity = match caller_access_key {
        Some(key) => self.state.resolve_identity(key),
        None => CallerIdentity::Root {
            account_id: self.config.default_account_id.clone(),
        },
    };

    let duration_seconds: i32 = get_optional_param(params, "DurationSeconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(43200); // Default 12 hours

    let gen = CredentialGenerator::new(identity.account_id().to_owned());
    let creds = gen.generate_temporary();
    let now = current_epoch_seconds();
    let expiration = now + f64::from(duration_seconds);

    // Store credential record with caller's identity.
    self.state.credentials.insert(
        creds.access_key_id.clone(),
        CredentialRecord {
            access_key_id: creds.access_key_id.clone(),
            secret_access_key: creds.secret_access_key.clone(),
            session_token: Some(creds.session_token.clone()),
            identity,
            expiration: Some(expiration),
        },
    );

    Ok(GetSessionTokenOutput {
        credentials: Credentials {
            access_key_id: creds.access_key_id,
            secret_access_key: creds.secret_access_key,
            session_token: creds.session_token,
            expiration: format_iso8601(expiration),
        },
    })
}
```

**GetAccessKeyInfo** -- Returns the account ID encoded in an access key.

Algorithm:
1. Extract `AccessKeyId` from request parameters.
2. First, check the credential store for a direct match.
3. If not found, decode the account ID from the access key's embedded encoding.
4. If decoding fails, return the default account ID.
5. Return `{ Account }`.

```rust
/// Handle GetAccessKeyInfo.
///
/// Returns the account ID associated with the given access key.
pub fn get_access_key_info(
    &self,
    params: &[(String, String)],
) -> Result<GetAccessKeyInfoOutput, StsError> {
    let access_key_id = get_required_param(params, "AccessKeyId")?;

    // Check credential store first.
    if let Some(record) = self.state.credentials.get(&access_key_id) {
        return Ok(GetAccessKeyInfoOutput {
            account: record.identity.account_id().to_owned(),
        });
    }

    // Decode account ID from the access key format.
    let account = account_id_from_access_key(&access_key_id)
        .unwrap_or_else(|| self.config.default_account_id.clone());

    Ok(GetAccessKeyInfoOutput { account })
}
```

#### Phase 1: Federation and Advanced (4 operations)

**AssumeRoleWithSAML** -- Returns temporary credentials using a SAML assertion.

Algorithm:
1. Extract `RoleArn`, `PrincipalArn`, and `SAMLAssertion` from parameters.
2. Validate `RoleArn` format.
3. Base64-decode the SAML assertion (do not validate the XML signature).
4. Extract role name and account ID from the role ARN.
5. Generate temporary credentials and session record (same as AssumeRole).
6. Return credentials, assumed role user, audience, issuer, subject, and SAML attributes.

For local development, the SAML assertion is accepted without cryptographic validation. The issuer and subject are extracted from the SAML XML if possible, or stubbed with defaults.

**AssumeRoleWithWebIdentity** -- Returns temporary credentials using an OIDC token.

Algorithm:
1. Extract `RoleArn`, `RoleSessionName`, and `WebIdentityToken` from parameters.
2. Validate `RoleArn` format and `RoleSessionName`.
3. Decode the JWT token (do not verify the signature).
4. Extract claims: `sub`, `aud`, `iss` from the JWT payload.
5. Generate temporary credentials and session record (same as AssumeRole).
6. Return credentials, assumed role user, audience, provider, subject from JWT.

For local development, the JWT is decoded but the signature is not verified against the IdP. The `sub` claim becomes the source identity.

**DecodeAuthorizationMessage** -- Decodes an authorization failure message.

Algorithm:
1. Accept `EncodedMessage` parameter.
2. Return a static JSON-formatted decoded message indicating the authorization context.
3. In local dev, the "decoded" message is simply the input passed through (since we do not encode authorization messages).

```rust
/// Handle DecodeAuthorizationMessage.
///
/// In local development, authorization messages are not actually encoded.
/// This operation returns a static decoded message.
pub fn decode_authorization_message(
    &self,
    params: &[(String, String)],
) -> Result<DecodeAuthorizationMessageOutput, StsError> {
    let encoded_message = get_required_param(params, "EncodedMessage")?;

    // Return the input as the "decoded" message, since we don't
    // actually encode authorization error messages.
    let decoded = serde_json::json!({
        "allowed": false,
        "explicitDeny": false,
        "matchedStatements": {
            "items": []
        },
        "failures": {
            "items": []
        },
        "context": {
            "principal": {
                "id": "AIDAQWERTYUIOPASDFGHJ",
                "arn": "arn:aws:iam::000000000000:user/local-user"
            },
            "action": "ec2:RunInstances",
            "resource": "arn:aws:ec2:us-east-1:000000000000:instance/*",
            "conditions": {
                "items": []
            }
        }
    });

    Ok(DecodeAuthorizationMessageOutput {
        decoded_message: decoded.to_string(),
    })
}
```

**GetFederationToken** -- Returns temporary credentials for a federated user.

Algorithm:
1. Extract `Name` (federated user name) from parameters.
2. Validate name: 2-32 characters, `[a-zA-Z_0-9+=,.@-]+`.
3. Extract optional `Policy` and `PolicyArns`.
4. Generate temporary credentials.
5. Create credential record with `CallerIdentity::FederatedUser`.
6. Return credentials, federated user ARN and ID, packed policy size.

### 10.3 Validation Rules

| Field | Rule |
|-------|------|
| RoleArn | Must match `arn:aws:iam::\d{12}:role/.+` |
| RoleSessionName | 2-64 chars, pattern `[a-zA-Z_0-9+=,.@-]+` |
| DurationSeconds (AssumeRole) | 900-43200 (15 min to 12 hours), default 3600 |
| DurationSeconds (GetSessionToken) | 900-129600 (15 min to 36 hours), default 43200 |
| DurationSeconds (Federation) | 900-129600 |
| ExternalId | 2-1224 chars, `[a-zA-Z0-9+=,.@:\\/-]+` |
| Policy (inline JSON) | Max 2048 chars (packed) |
| PolicyArns | Max 10 ARNs |
| Session tag key | 1-128 chars |
| Session tag value | 0-256 chars |
| Max session tags | 50 |
| Max transitive tag keys | 50 |
| SerialNumber (MFA) | Must match ARN pattern or `[0-9]{6,}` |
| Name (GetFederationToken) | 2-32 chars, `[a-zA-Z_0-9+=,.@-]+` |
| SourceIdentity | 2-64 chars, `[a-zA-Z_0-9+=,.@-]+` |

### 10.4 Form Parameter Parsing Helpers

The awsQuery protocol uses a specific convention for nested types:

```rust
/// Parse session tags from awsQuery form parameters.
///
/// Tags are encoded as:
/// - `Tags.member.1.Key=Project`
/// - `Tags.member.1.Value=MyProject`
/// - `Tags.member.2.Key=Env`
/// - `Tags.member.2.Value=Dev`
fn parse_session_tags(params: &[(String, String)]) -> Vec<SessionTag> {
    let mut tags = Vec::new();
    let mut index = 1;

    loop {
        let key_param = format!("Tags.member.{index}.Key");
        let value_param = format!("Tags.member.{index}.Value");

        let key = params
            .iter()
            .find(|(k, _)| k == &key_param)
            .map(|(_, v)| v.clone());
        let value = params
            .iter()
            .find(|(k, _)| k == &value_param)
            .map(|(_, v)| v.clone());

        match (key, value) {
            (Some(k), Some(v)) => {
                tags.push(SessionTag { key: k, value: v });
                index += 1;
            }
            _ => break,
        }
    }

    tags
}

/// Parse transitive tag keys from awsQuery form parameters.
///
/// Encoded as:
/// - `TransitiveTagKeys.member.1=Project`
/// - `TransitiveTagKeys.member.2=Env`
fn parse_transitive_tag_keys(params: &[(String, String)]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut index = 1;

    loop {
        let param = format!("TransitiveTagKeys.member.{index}");
        match params.iter().find(|(k, _)| k == &param) {
            Some((_, v)) => {
                keys.push(v.clone());
                index += 1;
            }
            None => break,
        }
    }

    keys
}

/// Parse policy ARNs from awsQuery form parameters.
fn parse_policy_arns(params: &[(String, String)]) -> Vec<String> {
    let mut arns = Vec::new();
    let mut index = 1;

    loop {
        let param = format!("PolicyArns.member.{index}.arn");
        match params.iter().find(|(k, _)| k == &param) {
            Some((_, v)) => {
                arns.push(v.clone());
                index += 1;
            }
            None => break,
        }
    }

    arns
}
```

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// STS error codes matching the AWS API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StsErrorCode {
    /// Missing required Action parameter.
    MissingAction,
    /// Invalid or unsupported action.
    InvalidAction,
    /// The security token included in the request is expired.
    ExpiredTokenException,
    /// The policy document is malformed.
    MalformedPolicyDocumentException,
    /// The IAM policy is too large (packed policy exceeds limit).
    PackedPolicyTooLargeException,
    /// The STS service is not activated in the requested region.
    RegionDisabledException,
    /// The identity token is invalid (WebIdentity/SAML).
    InvalidIdentityTokenException,
    /// The identity provider is not allowed for this operation.
    IDPRejectedClaimException,
    /// The IdP returned an error response.
    IDPCommunicationErrorException,
    /// The client token ID is not valid.
    InvalidClientTokenIdException,
    /// The request is missing required parameters or has invalid values.
    InvalidParameterValue,
    /// An internal error occurred.
    InternalError,
}

impl StsErrorCode {
    /// Return the HTTP status code for this error.
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::MissingAction
            | Self::InvalidAction
            | Self::MalformedPolicyDocumentException
            | Self::PackedPolicyTooLargeException
            | Self::InvalidIdentityTokenException
            | Self::IDPRejectedClaimException
            | Self::InvalidParameterValue => http::StatusCode::BAD_REQUEST,

            Self::ExpiredTokenException
            | Self::InvalidClientTokenIdException => http::StatusCode::FORBIDDEN,

            Self::RegionDisabledException => http::StatusCode::FORBIDDEN,

            Self::IDPCommunicationErrorException
            | Self::InternalError => http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Return the AWS error code string.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingAction => "MissingAction",
            Self::InvalidAction => "InvalidAction",
            Self::ExpiredTokenException => "ExpiredTokenException",
            Self::MalformedPolicyDocumentException => "MalformedPolicyDocument",
            Self::PackedPolicyTooLargeException => "PackedPolicyTooLarge",
            Self::RegionDisabledException => "RegionDisabledException",
            Self::InvalidIdentityTokenException => "InvalidIdentityToken",
            Self::IDPRejectedClaimException => "IDPRejectedClaim",
            Self::IDPCommunicationErrorException => "IDPCommunicationError",
            Self::InvalidClientTokenIdException => "InvalidClientTokenId",
            Self::InvalidParameterValue => "InvalidParameterValue",
            Self::InternalError => "InternalFailure",
        }
    }

    /// Return the fault type (Sender or Receiver).
    pub fn fault(&self) -> &'static str {
        match self {
            Self::InternalError | Self::IDPCommunicationErrorException => "Receiver",
            _ => "Sender",
        }
    }
}
```

### 11.2 Error Response Format

STS uses the standard awsQuery XML error format (same as SNS):

```xml
<ErrorResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <Error>
    <Type>Sender</Type>
    <Code>MalformedPolicyDocument</Code>
    <Message>The policy document is malformed.</Message>
  </Error>
  <RequestId>01234567-89ab-cdef-0123-456789abcdef</RequestId>
</ErrorResponse>
```

```rust
/// Build an XML error response.
pub fn error_to_response(error: &StsError, request_id: &str) -> http::Response<StsResponseBody> {
    let xml = format!(
        "<ErrorResponse xmlns=\"{STS_XML_NS}\">\
         <Error>\
         <Type>{}</Type>\
         <Code>{}</Code>\
         <Message>{}</Message>\
         </Error>\
         <RequestId>{}</RequestId>\
         </ErrorResponse>",
        error.code.fault(),
        error.code.code(),
        xml_escape(&error.message),
        xml_escape(request_id),
    );

    let body = StsResponseBody::from_xml(xml.into_bytes());
    http::Response::builder()
        .status(error.code.status_code())
        .header("content-type", "text/xml")
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid error response")
}
```

---

## 12. Server Integration

### 12.1 Feature Gate

STS support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns", "lambda", "events", "logs", "kms", "kinesis", "secretsmanager", "sts"]
sts = ["dep:rustack-sts-core", "dep:rustack-sts-http"]
```

### 12.2 Gateway Registration

STS must be registered **before** SNS in the gateway's router list, since both match form-urlencoded POST requests. STS matches only when the SigV4 service name is `sts`; SNS handles the remaining form-urlencoded requests.

```rust
// In gateway setup (build_services function in main.rs)
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

// JSON-protocol services (matched by X-Amz-Target header) - order doesn't matter
#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

#[cfg(feature = "ssm")]
services.push(Box::new(SsmServiceRouter::new(ssm_service)));

#[cfg(feature = "secretsmanager")]
services.push(Box::new(SecretsManagerServiceRouter::new(sm_service)));

// ... other X-Amz-Target services ...

// SQS (matched by X-Amz-Target: AmazonSQS)
#[cfg(feature = "sqs")]
services.push(Box::new(SqsServiceRouter::new(sqs_service)));

// STS (form-urlencoded + service=sts) -- MUST be before SNS
#[cfg(feature = "sts")]
services.push(Box::new(StsServiceRouter::new(sts_service)));

// SNS (form-urlencoded fallback)
#[cfg(feature = "sns")]
services.push(Box::new(SnsServiceRouter::new(sns_service)));

// Lambda (path-based routing)
#[cfg(feature = "lambda")]
services.push(Box::new(LambdaServiceRouter::new(lambda_service)));

// S3 is always last (catch-all for requests without other matches)
#[cfg(feature = "s3")]
services.push(Box::new(S3ServiceRouter::new(s3_service)));
```

### 12.3 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "ssm": "running",
        "sns": "running",
        "sts": "running"
    }
}
```

### 12.4 Configuration

```rust
/// STS service configuration.
#[derive(Debug, Clone)]
pub struct StsConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
    /// Default access key ID (maps to root of default account).
    pub default_access_key: String,
    /// Default secret access key.
    pub default_secret_key: String,
}

impl StsConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("STS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            default_access_key: env_str("AWS_ACCESS_KEY_ID", "test")
                .to_string(),
            default_secret_key: env_str("AWS_SECRET_ACCESS_KEY", "test")
                .to_string(),
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `STS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for STS |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for root credentials |
| `AWS_ACCESS_KEY_ID` | `test` | Default access key mapped to root |
| `AWS_SECRET_ACCESS_KEY` | `test` | Default secret key for root |

### 12.6 Integration with rustack-auth

The STS service has a bidirectional relationship with `rustack-auth`:

1. **Auth validates STS requests** -- like all other services, STS requests are authenticated via SigV4 using `rustack-auth`. The credential provider resolves access keys to secret keys.

2. **STS provides credentials to auth** -- when STS generates temporary credentials (via AssumeRole/GetSessionToken), those credentials need to be available to `rustack-auth` for validating subsequent requests signed with those temporary credentials.

For the initial implementation, this is handled through the shared `StaticCredentialProvider` or a new `DynamicCredentialProvider` that STS can register new credentials with:

```rust
/// A credential provider that supports dynamic registration of new credentials.
///
/// Used by STS to register temporary credentials so other services can
/// validate requests signed with those credentials.
pub trait DynamicCredentialProvider: CredentialProvider {
    /// Register a new access key / secret key pair.
    fn register_credential(&self, access_key: String, secret_key: String);
}
```

If signature validation is disabled (the default for local dev), this integration is not needed, and STS operates standalone.

### 12.7 Docker Image / GitHub Action

The existing Docker image and GitHub Action gain STS support automatically when the feature is enabled. The GitHub Action `action.yml` should be updated to list `sts` as a supported service.

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Credential generation**: ASIA prefix, correct length, base-36 account encoding/decoding round-trip
- **Identity resolution**: root identity for default key, root for unknown key, assumed-role identity for temporary key
- **Session tag propagation**: direct tags, transitive tag inheritance, tag override semantics, chained assume-role tag propagation
- **Role ARN validation**: valid ARNs, invalid format, missing account ID, path roles
- **Session name validation**: valid names, too short, too long, invalid characters
- **Access key account decoding**: encode account ID, decode it back, handle edge cases
- **Form parameter parsing**: tags with member.N pattern, transitive keys, policy ARNs, missing parameters

### 13.2 Integration Tests with aws-sdk-rust

```rust
// tests/integration/sts_tests.rs
#[tokio::test]
#[ignore]
async fn test_should_get_caller_identity_root() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client.get_caller_identity().send().await.unwrap();
    assert_eq!(result.account(), Some("000000000000"));
    assert_eq!(result.arn(), Some("arn:aws:iam::000000000000:root"));
}

#[tokio::test]
#[ignore]
async fn test_should_assume_role_and_get_credentials() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client
        .assume_role()
        .role_arn("arn:aws:iam::123456789012:role/TestRole")
        .role_session_name("test-session")
        .send()
        .await
        .unwrap();

    let creds = result.credentials().unwrap();
    assert!(creds.access_key_id().starts_with("ASIA"));
    assert!(!creds.secret_access_key().is_empty());
    assert!(!creds.session_token().is_empty());

    let assumed = result.assumed_role_user().unwrap();
    assert!(assumed.arn().contains("assumed-role/TestRole/test-session"));
}

#[tokio::test]
#[ignore]
async fn test_should_get_caller_identity_after_assume_role() {
    let sts = aws_sdk_sts::Client::new(&config);

    // Assume a role
    let assume_result = sts
        .assume_role()
        .role_arn("arn:aws:iam::123456789012:role/TestRole")
        .role_session_name("test-session")
        .send()
        .await
        .unwrap();

    let creds = assume_result.credentials().unwrap();

    // Create a new STS client with the temporary credentials
    let temp_config = build_config_with_credentials(
        creds.access_key_id(),
        creds.secret_access_key(),
        creds.session_token(),
    );
    let temp_sts = aws_sdk_sts::Client::new(&temp_config);

    // GetCallerIdentity should return the assumed role identity
    let identity = temp_sts.get_caller_identity().send().await.unwrap();
    assert_eq!(identity.account(), Some("123456789012"));
    assert!(identity.arn().unwrap().contains("assumed-role/TestRole/test-session"));
}

#[tokio::test]
#[ignore]
async fn test_should_propagate_transitive_tags() {
    let sts = aws_sdk_sts::Client::new(&config);

    // First AssumeRole with transitive tags
    let result1 = sts
        .assume_role()
        .role_arn("arn:aws:iam::123456789012:role/RoleA")
        .role_session_name("session-a")
        .tags(Tag::builder().key("Project").value("MyProject").build())
        .transitive_tag_keys("Project")
        .send()
        .await
        .unwrap();

    // Second AssumeRole using first role's credentials
    // The "Project" tag should propagate
    let creds1 = result1.credentials().unwrap();
    let temp_config = build_config_with_credentials(
        creds1.access_key_id(),
        creds1.secret_access_key(),
        creds1.session_token(),
    );
    let temp_sts = aws_sdk_sts::Client::new(&temp_config);

    let result2 = temp_sts
        .assume_role()
        .role_arn("arn:aws:iam::123456789012:role/RoleB")
        .role_session_name("session-b")
        .send()
        .await
        .unwrap();

    // Verify the chained session exists (transitive tags are internal state)
    assert!(result2.credentials().is_some());
}

#[tokio::test]
#[ignore]
async fn test_should_get_session_token() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client.get_session_token().send().await.unwrap();

    let creds = result.credentials().unwrap();
    assert!(creds.access_key_id().starts_with("ASIA"));
    assert!(!creds.secret_access_key().is_empty());
    assert!(!creds.session_token().is_empty());
}

#[tokio::test]
#[ignore]
async fn test_should_get_access_key_info() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client
        .get_access_key_info()
        .access_key_id("AKIAIOSFODNN7EXAMPLE")
        .send()
        .await
        .unwrap();

    assert!(result.account().is_some());
}

#[tokio::test]
#[ignore]
async fn test_should_reject_invalid_role_arn() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client
        .assume_role()
        .role_arn("not-a-valid-arn")
        .role_session_name("test")
        .send()
        .await;

    assert!(result.is_err());
}

#[tokio::test]
#[ignore]
async fn test_should_reject_invalid_session_name() {
    let client = aws_sdk_sts::Client::new(&config);
    let result = client
        .assume_role()
        .role_arn("arn:aws:iam::123456789012:role/TestRole")
        .role_session_name("x") // Too short (min 2)
        .send()
        .await;

    assert!(result.is_err());
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Get caller identity (most basic STS call)
aws sts get-caller-identity --endpoint-url http://localhost:4566

# Assume a role
aws sts assume-role \
  --role-arn arn:aws:iam::123456789012:role/TestRole \
  --role-session-name test-session \
  --endpoint-url http://localhost:4566

# Get session token
aws sts get-session-token --endpoint-url http://localhost:4566

# Get access key info
aws sts get-access-key-info \
  --access-key-id AKIAIOSFODNN7EXAMPLE \
  --endpoint-url http://localhost:4566

# Assume role with session tags
aws sts assume-role \
  --role-arn arn:aws:iam::123456789012:role/TestRole \
  --role-session-name tagged-session \
  --tags Key=Project,Value=MyProject Key=Environment,Value=Dev \
  --transitive-tag-keys Project \
  --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack STS Tests

**Location:** `vendors/localstack/tests/aws/services/sts/`
**Coverage:** Core operations:
- GetCallerIdentity (root, assumed role)
- AssumeRole (basic, with tags, with transitive tags, invalid ARN, invalid session name)
- Session tag propagation across chained AssumeRole calls
- GetSessionToken

#### 13.4.2 Moto STS Tests

**Source:** https://github.com/getmoto/moto/blob/master/tests/test_sts/
**Coverage:** All 8 operations including:
- AssumeRole, AssumeRoleWithSAML, AssumeRoleWithWebIdentity
- GetCallerIdentity, GetSessionToken, GetAccessKeyInfo
- GetFederationToken, DecodeAuthorizationMessage
- Session tag propagation
- Error cases

#### 13.4.3 Terraform AWS Provider

**What it validates:** Terraform calls `GetCallerIdentity` during `terraform init` and uses `AssumeRole` for provider configuration with `assume_role` blocks. This is the most common STS integration point.

```bash
# Verify Terraform works with Rustack STS
export AWS_ENDPOINT_URL=http://localhost:4566
terraform init    # Calls GetCallerIdentity
terraform plan    # Uses configured role if assume_role block is present
```

#### 13.4.4 AWS SDK Bootstrap Tests

Every AWS SDK calls `GetCallerIdentity` during client construction. Testing with multiple SDKs validates protocol compatibility:

```bash
# Python (boto3)
python3 -c "
import boto3
sts = boto3.client('sts', endpoint_url='http://localhost:4566')
print(sts.get_caller_identity())
"

# Node.js (aws-sdk-v3)
node -e "
const { STSClient, GetCallerIdentityCommand } = require('@aws-sdk/client-sts');
const client = new STSClient({ endpoint: 'http://localhost:4566' });
client.send(new GetCallerIdentityCommand({})).then(console.log);
"

# Go (aws-sdk-go-v2)
# Configure endpoint resolver to point to localhost:4566
```

### 13.5 CI Integration

```yaml
# .github/workflows/sts-ci.yml
name: STS CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-sts-model
      - run: cargo test -p rustack-sts-core
      - run: cargo test -p rustack-sts-http

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
          aws sts get-caller-identity --endpoint-url http://localhost:4566
          aws sts assume-role \
            --role-arn arn:aws:iam::123456789012:role/TestRole \
            --role-session-name ci-test \
            --endpoint-url http://localhost:4566
      - run: |
          # Python SDK test
          pip install boto3 pytest
          pytest tests/integration/sts/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: Core Operations (2-3 days)

**Goal:** `GetCallerIdentity` and `AssumeRole` work. Every AWS SDK can bootstrap against Rustack.

1. **Day 1: Model + Scaffolding + GetCallerIdentity**
   - Download STS Smithy model to `codegen/smithy-model/sts.json`
   - Create `codegen/services/sts.toml`
   - Generate `rustack-sts-model` crate
   - Create `rustack-sts-core` and `rustack-sts-http` crate scaffolding
   - Implement `StsOperation` enum and form-parameter router
   - Implement credential store and identity resolution
   - Implement `GetCallerIdentity` (root credentials)
   - Implement gateway routing (SigV4 service=sts disambiguation)
   - Integrate into gateway, health endpoint, feature gate

2. **Day 2: AssumeRole + GetSessionToken + GetAccessKeyInfo**
   - Implement temporary credential generation (ASIA keys, session tokens)
   - Implement `AssumeRole` with session creation and tag handling
   - Implement `GetSessionToken`
   - Implement `GetAccessKeyInfo` with account ID decoding
   - Implement `GetCallerIdentity` for assumed-role sessions

3. **Day 3: Tests + Tag Propagation + Polish**
   - Unit tests for all credential/session operations
   - Integration tests with aws-sdk-rust
   - AWS CLI smoke tests
   - Implement transitive tag propagation for chained AssumeRole
   - Fix edge cases from testing

**Deliverable:** AWS CLI, all AWS SDKs, Terraform init, CDK bootstrap all work against Rustack.

### Phase 1: Federation and Advanced (1-2 days)

**Goal:** Full STS API coverage including federation operations.

4. **Day 4: Federation Operations**
   - Implement `AssumeRoleWithSAML` (accept SAML assertion without validation)
   - Implement `AssumeRoleWithWebIdentity` (accept JWT without signature verification)
   - Implement `DecodeAuthorizationMessage` (stub)
   - Implement `GetFederationToken`

5. **Day 5: CI + Polish**
   - Integration tests for all 8 operations
   - Run LocalStack test suite subset
   - Update Docker image, GitHub Action, README
   - CI workflow for STS

**Deliverable:** All 8 operations implemented, CI green, Docker image updated.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Gateway routing conflict between STS and SNS | High | High | Parse SigV4 credential scope `service` field to disambiguate. Both are awsQuery with form-urlencoded. Without SigV4 parsing, requests would be misrouted. Test extensively with both STS and SNS enabled simultaneously. |
| SigV4 Authorization header absent or malformed | Medium | High | Some tools may send unsigned requests (especially in local dev). If Authorization header is missing, fall back to checking if the form body contains STS-specific Action names (GetCallerIdentity, AssumeRole, etc.) as a secondary routing heuristic. |
| Account ID encoding in access keys is incompatible | Low | Medium | The base-36 encoding is a local convention. `GetAccessKeyInfo` must handle both Rustack-generated keys (decodable) and external keys (default account fallback). Real AWS keys use a different encoding. |
| Temporary credentials not available to other services for auth | Medium | Medium | When signature validation is enabled, AssumeRole-generated credentials must be registered with the auth credential provider. Implement `DynamicCredentialProvider`. Not needed when validation is disabled (default). |
| Session token format breaks SDK expectations | Low | Low | AWS SDKs treat session tokens as opaque strings. No format validation is performed client-side. Our random tokens are safe. |
| Tag propagation semantics differ from real AWS | Medium | Medium | Transitive tag propagation has subtle rules around override priority and inheritance depth. Study LocalStack's implementation and AWS documentation carefully. Write extensive unit tests for chained assume-role scenarios. |
| GetCallerIdentity returns wrong ARN format | Medium | High | SDKs parse the ARN to determine account and identity type. Ensure root returns `arn:aws:iam::ACCOUNT:root` (not `user/moto` like moto's default). Ensure assumed roles return `arn:aws:sts::ACCOUNT:assumed-role/ROLE/SESSION`. |
| XML response format incompatible with SDK XML parsers | Low | Medium | AWS SDKs use strict XML parsers that expect exact element names and namespace. Test with aws-sdk-rust, boto3, and AWS CLI to validate XML format. Use the same `XmlWriter` patterns proven by the SNS implementation. |

### 15.2 Dependencies

- `rustack-core` -- no changes needed
- `rustack-auth` -- may need `DynamicCredentialProvider` trait for signature validation mode; no changes needed when validation is disabled
- `dashmap` -- already in workspace
- `uuid` -- for request IDs (already in workspace)
- `rand` -- for credential generation (already in workspace)
- `base64` -- for SAML assertion decoding in Phase 1 (already in workspace)
- `chrono` -- for ISO 8601 timestamp formatting in credential responses

### 15.3 Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Route STS by SigV4 `service=sts` in Authorization header | Only reliable way to distinguish STS from SNS at the gateway level. Both use identical wire format (awsQuery, form-urlencoded POST). X-Amz-Target is not used by either service. |
| Permissive identity resolution (unknown keys = root) | Matches LocalStack behavior. In local dev, any access key should work. Strictness is opt-in via signature validation. |
| Encode account ID in access key (base-36 in chars 4-7) | Enables `GetAccessKeyInfo` to return the correct account without a store lookup. Not compatible with real AWS key encoding, but sufficient for local dev. |
| Session tokens are random opaque strings | No cryptographic signing needed for local dev. SDKs treat tokens as opaque. Simpler than implementing real STS token format. |
| Tag propagation tracked in session store | Transitive tags must survive across chained AssumeRole calls. Storing them in the session record (keyed by session token) allows the next AssumeRole to look up the caller's tags by their access key -> session token -> session record chain. |
| Credential expiration not enforced | Simplifies local dev. Real AWS enforces expiration, but for testing, perpetual credentials avoid spurious auth failures during long-running tests. |
| STS is a global store (not per-region) | Real AWS STS has regional endpoints but a global identity model. For local dev, a single store is sufficient and avoids complexity. |
| Optional IAM role validation | STS should work even when IAM is not enabled. Role ARN format is validated, but role existence is not checked unless IAM is available. This allows standalone STS usage. |
