# Rustack SES: Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-sns-design.md](./rustack-sns-design.md)
**Scope:** Add AWS Simple Email Service (SES) v1 support to Rustack as the primary implementation, plus core SES v2 operations. SES v1 uses the `awsQuery` protocol (same as SNS); SES v2 uses `restJson1`. The implementation captures all sent emails for retrospection via a `/_aws/ses` REST endpoint -- the primary value for local development and testing.

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
9. [Storage Engine Design](#9-storage-engine-design)
10. [Core Business Logic](#10-core-business-logic)
11. [Error Handling](#11-error-handling)
12. [Server Integration](#12-server-integration)
13. [Testing Strategy](#13-testing-strategy)
14. [Phased Implementation Plan](#14-phased-implementation-plan)
15. [Risk Analysis](#15-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding AWS Simple Email Service (SES) support to Rustack. Key points:

- **Universally needed** -- virtually every application sends emails, and SES is AWS's primary email-sending service. Adding SES unlocks local development and CI testing for any application that sends transactional emails, password resets, notifications, or marketing campaigns via AWS SES.
- **Small API surface, high testing value** -- SES v1 has ~30 operations, but the core sending operations (SendEmail, SendRawEmail, SendTemplatedEmail) plus identity management cover 90%+ of local development needs. The implementation is simpler than services like DynamoDB or Secrets Manager because there is no complex state machine or versioning semantics.
- **Email retrospection endpoint is the killer feature** -- the `/_aws/ses` REST endpoint allows tests to query all "sent" emails after calling the SES API. Tests send emails via the standard SES SDK, then assert on email content, recipients, and subjects via this endpoint. This is the primary differentiator from simply stubbing the API.
- **No actual email delivery** -- Rustack never sends real emails. All emails are captured in memory and exposed via the retrospection endpoint. This makes it safe for CI/CD, development, and testing without risk of accidental email delivery.
- **Dual protocol: awsQuery (v1) + restJson1 (v2)** -- SES v1 uses `awsQuery` (identical to SNS), which is still the default for many tools. SES v2 uses `restJson1` with path-based routing under `/v2/email/`. We implement v1 as the primary API and add core v2 operations as a stretch goal.
- **awsQuery protocol reuse** -- SES v1 uses the same `awsQuery` protocol as SNS: `application/x-www-form-urlencoded` request bodies with XML responses. The gateway routes SES v1 by inspecting the SigV4 `Credential` service name (`email` for SES) since the Content-Type and request shape overlap with SNS.
- **Estimated effort** -- 3-4 days for Phase 0 (12 core operations), 6-8 days for full implementation (all 4 phases).

---

## 2. Motivation

### 2.1 Why SES?

AWS SES is the standard email-sending service for AWS-hosted applications. Every non-trivial application sends email:

- **Transactional emails** -- password resets, email verification, order confirmations, shipping notifications
- **Notifications** -- alert emails, digest emails, system notifications
- **Marketing emails** -- campaigns, newsletters (via templates and configuration sets)
- **Application testing** -- end-to-end tests that verify "did we send the right email to the right person with the right content?"

Without a local SES emulator, developers must either:
1. Skip email testing entirely (common, dangerous)
2. Use a real SES sandbox (requires AWS credentials, slow, flaky in CI)
3. Mock the SES SDK client in application code (misses integration bugs)
4. Use a third-party SMTP capture tool like MailHog/MailPit (doesn't test the SES API surface)

Rustack's SES fills this gap: applications use the real AWS SDK to send emails, and tests query the retrospection endpoint to verify email content. No code changes needed in the application under test.

### 2.2 Complexity Assessment

| Dimension | SES v1 | SNS | SSM | Secrets Manager |
|-----------|--------|-----|-----|-----------------|
| Total operations | ~30 | ~40 | 13 | 23 |
| MVP operations | 12 | 14 | 13 | 10 |
| Complex state machines | 0 | 1 (delivery) | 0 | 1 (rotation) |
| Storage complexity | Simple hashmap per store | Topic/sub graph | HashMap + BTreeMap | HashMap + version map |
| Concurrency model | Request/response | Pub/sub fan-out | Request/response | Request/response |
| Protocol | awsQuery (reuse SNS) | awsQuery (exists) | awsJson1.1 | awsJson1.1 |
| Cross-service integration | Optional SNS events | SQS fan-out | None | None |
| Estimated lines of code | ~3,000 | ~6,000 | ~3,000 | ~4,500 |

SES is one of the simpler services to implement because:
1. No complex versioning or state machines
2. Storage is append-only for sent emails (no updates, no transactions)
3. Identity verification always succeeds in local dev
4. Templates are simple key-value stores with basic substitution
5. The `awsQuery` protocol infrastructure already exists from SNS

### 2.3 Tool Coverage

With all 4 phases implemented, the following tools work out of the box:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws ses`) | SendEmail, VerifyEmailIdentity, ListIdentities, templates | Phase 0 + Phase 1 |
| AWS CLI (`aws sesv2`) | SendEmail (v2), CreateEmailIdentity, GetAccount | Phase 3 |
| Terraform (`aws_ses_email_identity`, `aws_ses_template`) | VerifyEmailIdentity, CreateTemplate, CreateConfigurationSet | Phase 0 + Phase 1 |
| AWS CDK | SendEmail, VerifyEmailIdentity, templates | Phase 0 + Phase 1 |
| AWS SDK (any language) | All sending operations | Phase 0 |
| Spring Cloud AWS | SendEmail, SendTemplatedEmail | Phase 0 |
| Django SES backend | SendRawEmail, SendEmail | Phase 0 |
| Nodemailer (AWS transport) | SendRawEmail | Phase 0 |
| Laravel SES driver | SendRawEmail | Phase 0 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Core sending operations** -- SendEmail, SendRawEmail, SendTemplatedEmail with full request validation and email capture
2. **Email retrospection endpoint** -- `GET /_aws/ses` to list all sent emails (filterable by id, source), `DELETE /_aws/ses` to clear all. This is the primary value proposition.
3. **Identity management** -- VerifyEmailIdentity, VerifyDomainIdentity, ListIdentities, DeleteIdentity, GetIdentityVerificationAttributes. All verifications succeed immediately (auto-verify for local dev).
4. **Template management** -- CreateTemplate, GetTemplate, UpdateTemplate, DeleteTemplate, ListTemplates. Templates support basic `{{variable}}` substitution for SendTemplatedEmail.
5. **Configuration sets** -- CreateConfigurationSet, DeleteConfigurationSet, DescribeConfigurationSet, ListConfigurationSets. Accept and store.
6. **Configuration set event destinations** -- CreateConfigurationSetEventDestination, UpdateConfigurationSetEventDestination, DeleteConfigurationSetEventDestination. Accept and store; future: emit events to SNS.
7. **Receipt rule sets** -- CreateReceiptRuleSet, DeleteReceiptRuleSet, CreateReceiptRule, DeleteReceiptRule, DescribeReceiptRuleSet, CloneReceiptRuleSet. Accept and store (do not actually receive email).
8. **Send statistics** -- GetSendQuota, GetSendStatistics. Track send count and return configurable quota.
9. **Tag validation** -- validate message tags on SendEmail per AWS rules (alphanumeric, max 255 chars)
10. **SES v2 core operations** -- SendEmail (v2), CreateEmailIdentity, GetEmailIdentity, DeleteEmailIdentity, ListEmailIdentities, CreateEmailTemplate, GetEmailTemplate, GetAccount, PutAccountDetails. Shares underlying stores with v1.
11. **Smithy-generated types** -- all types generated from official AWS Smithy model for SES v1
12. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsQuery protocol layer from SNS
13. **Same Docker image** -- single binary serves all existing services + SES on port 4566

### 3.2 Non-Goals

1. **Real email delivery** -- never send actual emails. The entire point is to capture emails for test assertions.
2. **DKIM/SPF verification** -- accept DKIM and SPF operations, return success stubs
3. **Bounce/complaint simulation** -- do not simulate bounces, complaints, or delivery failures
4. **SES v2 full API** -- SES v2 has 100+ operations. We implement ~10 core operations; remaining v2 operations are out of scope.
5. **Sending limits enforcement** -- GetSendQuota returns configurable values but does not actually rate-limit sends
6. **Suppression list** -- accept but do not enforce the account-level suppression list
7. **Dedicated IP pools** -- not applicable to local dev
8. **Mail-from domain configuration** -- accept and store, do not enforce
9. **Cross-account sending** -- no cross-account SES authorization
10. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
11. **Real receipt rule processing** -- receipt rules accepted and stored but email is never actually received by SES

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                AWS SDK / CLI / Terraform / Django / Spring
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  Routes by X-Amz-Target, Content-Type,
              |   (ServiceRouter)   |  SigV4 service, path prefix
              +--------+------------+
                       |
         +------+------+------+------+------+------+------+
         v      v      v      v      v      v      v      v
   +------+ +-----+ +-----+ +-----+ +------+ +-----+ +-----+
   | S3   | | DDB | | SQS | | SSM | | SNS  | | ... | | SES |
   |(restXml|(J10)| |(Qry)| |(J11)| |(Qry) | |     | |(Qry)|
   +------+ +-----+ +-----+ +-----+ +------+ +-----+ +-----+
       |        |        |       |       |               |
   +------+ +-----+ +-----+ +-----+ +------+        +------+
   |S3Core| |DDB  | |SQS  | |SSM  | |SNS   |        |SES   |
   |      | |Core | |Core | |Core | |Core   |        |Core  |
   +------+ +-----+ +-----+ +-----+ +------+        +------+
       |        |        |       |       |               |
       +--------+--------+-------+-------+---------------+
                         |
                  +------+------+
                  | rustack-  |
                  | core + auth |
                  +-------------+

              +---------------------------------------------+
              |         /_aws/ses endpoint                   |
              |   (Email retrospection REST API)             |
              +---------------------------------------------+
```

### 4.2 Gateway Routing

SES presents a unique routing challenge because SES v1 uses `awsQuery` (like SNS), with `POST /` and `Content-Type: application/x-www-form-urlencoded`. The gateway must distinguish SES from SNS when both use form-urlencoded POST requests.

**Routing strategy: SigV4 Credential service name**

The SigV4 `Authorization` header contains the service name in the `Credential` field:

```
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/email/aws4_request, ...
```

For SES v1, the service name is `email`. For SNS, the service name is `sns`. The gateway can parse this from the `Authorization` header to disambiguate.

| Service | Protocol | Content-Type | X-Amz-Target | SigV4 Service | Path |
|---------|----------|-------------|--------------|---------------|------|
| SNS | awsQuery | `x-www-form-urlencoded` | absent | `sns` | `/` |
| SES v1 | awsQuery | `x-www-form-urlencoded` | absent | `email` | `/` |
| SES v2 | restJson1 | `application/json` | absent | `ses` | `/v2/email/*` |
| SQS | awsQuery/awsJson | varies | `AmazonSQS.*` or absent | `sqs` | `/` or `/{account}/{queue}` |

**Routing logic** (evaluated in order):

1. If `X-Amz-Target` starts with known prefixes (DynamoDB, SSM, SQS, Kinesis, KMS, Logs, Secrets Manager, EventBridge) -- route to that service
2. If path starts with `/v2/email/` -- route to SES v2
3. If `Content-Type` is `x-www-form-urlencoded` and `POST /`:
   - Parse SigV4 `Credential` service name from `Authorization` header
   - If service is `email` -- route to SES v1
   - Otherwise -- route to SNS (existing behavior)
4. If path matches Lambda API patterns (`/2015-03-31/functions/`, etc.) -- route to Lambda
5. Default: route to S3 (catch-all)

**SES v2 routing** is simpler: match `path.starts_with("/v2/email/")`. SES v2 uses `restJson1` with JSON request/response bodies and path-based operation dispatch.

**Email retrospection endpoint**: `/_aws/ses` is handled at the gateway level (similar to health checks) or as a dedicated internal route:
- `GET /_aws/ses` -- list sent emails (optional query params: `id`, `email`)
- `DELETE /_aws/ses` -- clear all sent emails (optional query param: `id` to delete specific email)

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
+-- rustack-ses-model             <-- NEW (auto-generated from SES v1 Smithy model)
+-- rustack-ses-core              <-- NEW
+-- rustack-ses-http              <-- NEW
+-- ... (other services)

rustack-ses-http
+-- rustack-ses-model
+-- rustack-auth
+-- quick-xml (XML response serialization, reuse from SNS)
+-- serde_urlencoded (form request deserialization, reuse from SNS)
+-- serde_json (for SES v2 restJson1)

rustack-ses-core
+-- rustack-core
+-- rustack-ses-model
+-- dashmap
+-- serde_json (for template data, retrospection API)
+-- tracing
+-- uuid

rustack-ses-model (auto-generated, standalone)
+-- serde
+-- serde_json
```

---

## 5. Protocol Design

### 5.1 SES v1: awsQuery Protocol

SES v1 uses the `@awsQuery` protocol with API version `2010-12-01`. This is structurally identical to SNS's awsQuery protocol. All AWS SDKs send SES v1 requests as `application/x-www-form-urlencoded` and receive XML responses.

| Aspect | SES v1 (awsQuery) | SNS (awsQuery) |
|--------|-------------------|----------------|
| Content-Type (request) | `application/x-www-form-urlencoded` | `application/x-www-form-urlencoded` |
| Content-Type (response) | `text/xml` | `text/xml` |
| HTTP Method | POST | POST |
| URL Path | `/` | `/` |
| Operation dispatch | `Action=<OperationName>` form parameter | `Action=<OperationName>` form parameter |
| API version | `2010-12-01` | `2010-03-31` |
| XML namespace | `http://ses.amazonaws.com/doc/2010-12-01/` | `http://sns.amazonaws.com/doc/2010-03-31/` |
| SigV4 service | `email` | `sns` |

The only differences are the API version, XML namespace, SigV4 service name, and the set of `Action` values. The form parsing, XML serialization, and error formatting are identical.

### 5.2 SES v1 Wire Format Examples

**SendEmail request:**

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/email/aws4_request, ...

Action=SendEmail
&Source=sender%40example.com
&Destination.ToAddresses.member.1=recipient%40example.com
&Message.Subject.Data=Test+Subject
&Message.Body.Text.Data=Hello+World
&Message.Body.Html.Data=%3Cp%3EHello+World%3C%2Fp%3E
&Version=2010-12-01
```

**SendEmail response:**

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<SendEmailResponse xmlns="http://ses.amazonaws.com/doc/2010-12-01/">
  <SendEmailResult>
    <MessageId>00000000-1111-2222-3333-444444444444</MessageId>
  </SendEmailResult>
  <ResponseMetadata>
    <RequestId>aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</RequestId>
  </ResponseMetadata>
</SendEmailResponse>
```

**Error response:**

```http
HTTP/1.1 400 Bad Request
Content-Type: text/xml

<ErrorResponse xmlns="http://ses.amazonaws.com/doc/2010-12-01/">
  <Error>
    <Type>Sender</Type>
    <Code>MessageRejected</Code>
    <Message>Email address is not verified.</Message>
  </Error>
  <RequestId>aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</RequestId>
</ErrorResponse>
```

### 5.3 SES v2: restJson1 Protocol

SES v2 uses `restJson1` with path-based routing under `/v2/email/`. This is simpler than awsQuery -- JSON request/response bodies with HTTP method + path determining the operation.

| Aspect | SES v2 (restJson1) |
|--------|--------------------|
| Content-Type (request) | `application/json` |
| Content-Type (response) | `application/json` |
| HTTP Methods | GET, POST, PUT, DELETE |
| URL Path | `/v2/email/{resource}` |
| SigV4 service | `ses` |

**SES v2 path-to-operation mapping:**

| Method | Path | Operation |
|--------|------|-----------|
| POST | `/v2/email/identities` | CreateEmailIdentity |
| GET | `/v2/email/identities/{EmailIdentity}` | GetEmailIdentity |
| DELETE | `/v2/email/identities/{EmailIdentity}` | DeleteEmailIdentity |
| GET | `/v2/email/identities` | ListEmailIdentities |
| POST | `/v2/email/outbound-emails` | SendEmail (v2) |
| POST | `/v2/email/templates` | CreateEmailTemplate |
| GET | `/v2/email/templates/{TemplateName}` | GetEmailTemplate |
| GET | `/v2/email/account` | GetAccount |
| POST | `/v2/email/account/details` | PutAccountDetails |

**SES v2 SendEmail request:**

```http
POST /v2/email/outbound-emails HTTP/1.1
Content-Type: application/json
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/ses/aws4_request, ...

{
  "FromEmailAddress": "sender@example.com",
  "Destination": {
    "ToAddresses": ["recipient@example.com"]
  },
  "Content": {
    "Simple": {
      "Subject": { "Data": "Test Subject" },
      "Body": {
        "Text": { "Data": "Hello World" },
        "Html": { "Data": "<p>Hello World</p>" }
      }
    }
  }
}
```

**SES v2 SendEmail response:**

```json
{
  "MessageId": "00000000-1111-2222-3333-444444444444"
}
```

### 5.4 What We Reuse from SNS

The SNS implementation provides the core awsQuery infrastructure that SES v1 needs:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| Form-urlencoded request parsing | Yes | `serde_urlencoded` + custom nested-param parser |
| XML response serialization | Yes | `quick-xml` with response wrapper pattern |
| XML error response formatting | Yes | Same `<ErrorResponse>` structure |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| Multi-account/region state | Yes | `rustack-core` unchanged |
| Form parameter list/map encoding | Yes | `member.N.Key`/`member.N.Value` patterns |

The SES HTTP crate can import or duplicate the XML serialization helpers from the SNS HTTP crate. If the code is substantial enough, consider extracting into a shared `rustack-query-protocol` crate.

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-ses-model` crate is generated from the official AWS SES v1 Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/ses.json` (namespace `com.amazonaws.ses`)
**Service config:** `codegen/services/ses.toml`
**Generate:** `make codegen-ses`

The SES v1 Smithy model file needs to be obtained from the official AWS API models repository (`aws/aws-models` or `aws/api-models-aws`). The model file is `ses-2010-12-01.json`.

### 6.2 Proposed `codegen/services/ses.toml`

```toml
[service]
name = "ses"
display_name = "SES"
rust_prefix = "Ses"
namespace = "com.amazonaws.ses"
protocol = "awsQuery"

[protocol]
serde_rename = "PascalCase"
emit_serde_derives = true

[operations]
phase0 = [
    # Identity management
    "VerifyEmailIdentity", "VerifyDomainIdentity", "ListIdentities",
    "DeleteIdentity", "GetIdentityVerificationAttributes",
    "VerifyEmailAddress", "DeleteVerifiedEmailAddress", "ListVerifiedEmailAddresses",
    # Sending
    "SendEmail", "SendRawEmail",
    # Statistics
    "GetSendQuota", "GetSendStatistics",
]
phase1 = [
    # Templates
    "CreateTemplate", "GetTemplate", "UpdateTemplate", "DeleteTemplate", "ListTemplates",
    "SendTemplatedEmail",
    # Configuration sets
    "CreateConfigurationSet", "DeleteConfigurationSet",
    "DescribeConfigurationSet", "ListConfigurationSets",
]
phase2 = [
    # Configuration set event destinations
    "CreateConfigurationSetEventDestination",
    "UpdateConfigurationSetEventDestination",
    "DeleteConfigurationSetEventDestination",
    # Receipt rules
    "CreateReceiptRuleSet", "DeleteReceiptRuleSet",
    "CreateReceiptRule", "DeleteReceiptRule",
    "DescribeReceiptRuleSet", "CloneReceiptRuleSet",
    "DescribeActiveReceiptRuleSet", "SetActiveReceiptRuleSet",
]
phase3 = [
    # Identity notification configuration
    "SetIdentityNotificationTopic", "SetIdentityFeedbackForwardingEnabled",
    "GetIdentityNotificationAttributes",
    # Identity DKIM
    "VerifyDomainDkim", "GetIdentityDkimAttributes",
    # Identity mail-from
    "SetIdentityMailFromDomain", "GetIdentityMailFromDomainAttributes",
    # Sending authorization
    "GetIdentityPolicies", "PutIdentityPolicy", "DeleteIdentityPolicy", "ListIdentityPolicies",
]

[errors.custom]
MessageRejected = { status = 400, message = "Email address is not verified." }
ConfigurationSetDoesNotExist = { status = 400, message = "Configuration set does not exist." }
TemplateDoesNotExist = { status = 400, message = "Template does not exist." }
AlreadyExists = { status = 400, message = "Resource already exists." }
InvalidParameterValue = { status = 400, message = "Invalid parameter value." }
RuleSetDoesNotExist = { status = 400, message = "Rule set does not exist." }
RuleDoesNotExist = { status = 400, message = "Rule does not exist." }
LimitExceeded = { status = 400, message = "Limit exceeded." }
InvalidTemplate = { status = 400, message = "Invalid template." }

[output]
file_layout = "flat"
```

### 6.3 Generated Output

The codegen produces 6 files in `crates/rustack-ses-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs) with serde derives |
| `operations.rs` | `SesOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `SesErrorCode` enum + `SesError` struct + `ses_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.4 Service-Specific Notes

The awsQuery protocol uses a nested form-parameter encoding for complex types. For example, `Destination.ToAddresses.member.1=foo@bar.com`. The codegen produces flat Rust structs; the HTTP layer handles the form-to-struct deserialization (reusing the SNS pattern).

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-ses-model` (auto-generated)

```
crates/rustack-ses-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: SesOperation enum
    +-- error.rs            # Auto-generated: error types + error codes
    +-- input.rs            # Auto-generated: all input structs
    +-- output.rs           # Auto-generated: all output structs
```

**Dependencies:** `serde`, `serde_json`

### 7.2 `rustack-ses-core`

```
crates/rustack-ses-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # SesConfig
    +-- handler.rs          # SesHandler trait (all operation dispatch)
    +-- provider.rs         # RustackSes (main provider, all operation handlers)
    +-- storage.rs          # All stores: IdentityStore, EmailStore, TemplateStore, etc.
    +-- identity.rs         # Identity verification logic
    +-- template.rs         # Template rendering ({{variable}} substitution)
    +-- retrospection.rs    # SentEmail type, email capture, query/filter logic
    +-- statistics.rs       # Send statistics and quota tracking
    +-- validation.rs       # Tag validation, email address validation
    +-- config_set.rs       # Configuration set and event destination management
    +-- receipt_rule.rs     # Receipt rule set management
```

**Dependencies:** `rustack-core`, `rustack-ses-model`, `dashmap`, `serde_json`, `tracing`, `uuid`, `chrono`

### 7.3 `rustack-ses-http`

```
crates/rustack-ses-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # Action= parameter dispatch for SES v1
    +-- service.rs          # SesHttpService (hyper Service impl)
    +-- dispatch.rs         # SesHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- response.rs         # XML response construction (reuse SNS patterns)
    +-- v2/                 # SES v2 restJson1 handler
        +-- mod.rs
        +-- router.rs       # Path-based routing for /v2/email/*
        +-- handlers.rs     # Individual v2 operation handlers
```

**Dependencies:** `rustack-ses-model`, `rustack-auth`, `hyper`, `http`, `serde_json`, `serde_urlencoded`, `quick-xml`, `bytes`

This crate is structurally similar to `rustack-sns-http`. The router parses `Action=<SesOperation>` from the form body (v1) or matches path patterns (v2).

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-ses-model = { path = "crates/rustack-ses-model" }
rustack-ses-http = { path = "crates/rustack-ses-http" }
rustack-ses-core = { path = "crates/rustack-ses-core" }
```

---

## 8. HTTP Layer Design

### 8.1 SES v1 Router (awsQuery)

```rust
/// SES v1 operation router.
///
/// Parses the `Action=<Op>` form parameter from the request body
/// to determine the SES operation.
pub struct SesQueryRouter;

impl SesQueryRouter {
    pub fn resolve(action: &str) -> Result<SesOperation, SesError> {
        SesOperation::from_name(action)
            .ok_or_else(|| SesError::invalid_action(action))
    }
}
```

### 8.2 SES v2 Router (restJson1)

```rust
/// SES v2 operation router.
///
/// Matches HTTP method + path pattern under `/v2/email/` to determine
/// the SES v2 operation.
pub struct SesV2Router;

impl SesV2Router {
    pub fn resolve(method: &http::Method, path: &str) -> Option<SesV2Operation> {
        let path = path.strip_prefix("/v2/email")?;

        match (method, path) {
            (&Method::POST, "/identities") => Some(SesV2Operation::CreateEmailIdentity),
            (&Method::GET, "/identities") => Some(SesV2Operation::ListEmailIdentities),
            (&Method::GET, p) if p.starts_with("/identities/") => {
                Some(SesV2Operation::GetEmailIdentity)
            }
            (&Method::DELETE, p) if p.starts_with("/identities/") => {
                Some(SesV2Operation::DeleteEmailIdentity)
            }
            (&Method::POST, "/outbound-emails") => Some(SesV2Operation::SendEmail),
            (&Method::POST, "/templates") => Some(SesV2Operation::CreateEmailTemplate),
            (&Method::GET, p) if p.starts_with("/templates/") => {
                Some(SesV2Operation::GetEmailTemplate)
            }
            (&Method::GET, "/account") => Some(SesV2Operation::GetAccount),
            (&Method::POST, "/account/details") => Some(SesV2Operation::PutAccountDetails),
            _ => None,
        }
    }
}
```

### 8.3 ServiceRouter Trait Implementation

```rust
/// SES service router for the gateway.
///
/// Routes SES v1 (awsQuery) requests based on SigV4 service name `email`
/// and form-urlencoded Content-Type.
/// Routes SES v2 (restJson1) requests based on `/v2/email/` path prefix.
pub struct SesServiceRouter<H: SesHandler> {
    inner_v1: SesHttpService<H>,
    inner_v2: SesV2HttpService<H>,
}

impl<H: SesHandler> ServiceRouter for SesServiceRouter<H> {
    fn name(&self) -> &'static str {
        "ses"
    }

    /// SES matches in two ways:
    /// 1. SES v2: path starts with `/v2/email/`
    /// 2. SES v1: form-urlencoded POST with SigV4 service=`email`
    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        // SES v2: path-based
        if req.uri().path().starts_with("/v2/email/") {
            return true;
        }

        // SES v1: form-urlencoded POST with SigV4 service=email
        if *req.method() != http::Method::POST {
            return false;
        }

        let is_form = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("x-www-form-urlencoded"));

        if !is_form {
            return false;
        }

        // Check SigV4 Credential for service=email
        extract_sigv4_service(req.headers())
            .is_some_and(|svc| svc == "email")
    }

    async fn handle(&self, req: Request<Incoming>) -> Response<Body> {
        if req.uri().path().starts_with("/v2/email/") {
            self.inner_v2.call(req).await
        } else {
            self.inner_v1.call(req).await
        }
    }
}

/// Extract the SigV4 service name from the Authorization header.
///
/// Parses `Credential=AKID/date/region/SERVICE/aws4_request` and returns SERVICE.
fn extract_sigv4_service(headers: &http::HeaderMap) -> Option<&str> {
    let auth = headers.get("authorization")?.to_str().ok()?;
    let credential_start = auth.find("Credential=")? + "Credential=".len();
    let credential_end = auth[credential_start..].find(',')
        .map(|i| credential_start + i)
        .unwrap_or(auth.len());
    let credential = &auth[credential_start..credential_end];
    // Format: AKID/date/region/service/aws4_request
    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() >= 4 {
        Some(parts[3])
    } else {
        None
    }
}
```

### 8.4 Retrospection Endpoint Handler

```rust
/// Handler for the `/_aws/ses` email retrospection endpoint.
///
/// This is the primary value of the SES implementation for testing.
/// Tests send emails via the SES API, then query this endpoint to verify
/// email content.
pub struct SesRetrospectionHandler {
    email_store: Arc<EmailStore>,
}

impl SesRetrospectionHandler {
    /// GET /_aws/ses -- list sent emails.
    ///
    /// Query parameters:
    /// - `id`: filter by message ID
    /// - `email`: filter by source email address
    pub async fn handle_get(
        &self,
        query_params: &HashMap<String, String>,
    ) -> http::Response<GatewayBody> {
        let filter_id = query_params.get("id");
        let filter_source = query_params.get("email");

        let messages = self.email_store.query(filter_id, filter_source);

        let body = serde_json::json!({
            "messages": messages
        });

        http::Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(body.to_string().into())
            .expect("static response should be valid")
    }

    /// DELETE /_aws/ses -- clear sent emails.
    ///
    /// Query parameters:
    /// - `id`: delete specific email by message ID (if absent, clear all)
    pub async fn handle_delete(
        &self,
        query_params: &HashMap<String, String>,
    ) -> http::Response<GatewayBody> {
        if let Some(id) = query_params.get("id") {
            self.email_store.remove(id);
        } else {
            self.email_store.clear();
        }

        http::Response::builder()
            .status(204)
            .body(GatewayBody::empty())
            .expect("static response should be valid")
    }
}
```

### 8.5 Handler Trait

```rust
/// Trait that the SES business logic provider must implement.
pub trait SesHandler: Send + Sync + 'static {
    /// Handle an SES v1 operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: SesOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<
        http::Response<SesResponseBody>,
        SesError,
    >> + Send>>;

    /// Get access to the email store for the retrospection endpoint.
    fn email_store(&self) -> &Arc<EmailStore>;
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

SES requires five distinct stores, all relatively simple compared to services like DynamoDB or Secrets Manager. The primary store (EmailStore) is append-only for captured emails. The others are basic key-value stores for identities, templates, configuration sets, and receipt rules.

### 9.2 Core Data Structures

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;

/// Top-level SES state container.
/// Keyed by (account_id, region) via rustack-core.
pub struct SesState {
    pub identities: IdentityStore,
    pub emails: EmailStore,
    pub templates: TemplateStore,
    pub configuration_sets: ConfigurationSetStore,
    pub receipt_rule_sets: ReceiptRuleSetStore,
    pub statistics: SendStatistics,
    pub account: AccountState,
}
```

### 9.3 IdentityStore

```rust
/// Store for verified email addresses and domains.
///
/// In local development mode, all identities are auto-verified on creation.
/// In strict mode (configurable), identities must be explicitly verified first
/// before they can be used as a source address in SendEmail.
pub struct IdentityStore {
    /// All identities keyed by identity string (email address or domain).
    /// Value is the verification status.
    identities: DashMap<String, IdentityRecord>,
}

/// A single verified identity (email address or domain).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRecord {
    /// The identity string (email address or domain name).
    pub identity: String,
    /// Type of identity.
    pub identity_type: IdentityType,
    /// Verification status (always Success in local dev).
    pub verification_status: VerificationStatus,
    /// Verification token (for domain identities).
    pub verification_token: Option<String>,
    /// DKIM attributes (stub).
    pub dkim_enabled: bool,
    /// Notification topic ARNs.
    pub notification_topics: HashMap<String, Option<String>>,
    /// Feedback forwarding enabled.
    pub feedback_forwarding_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub enum IdentityType {
    EmailAddress,
    Domain,
}

#[derive(Debug, Clone, Serialize)]
pub enum VerificationStatus {
    Pending,
    Success,
    Failed,
    TemporaryFailure,
    NotStarted,
}

impl IdentityStore {
    pub fn new() -> Self {
        Self {
            identities: DashMap::new(),
        }
    }

    /// Add an email identity. Auto-verifies in local dev mode.
    pub fn verify_email(&self, email: &str) -> IdentityRecord {
        let record = IdentityRecord {
            identity: email.to_string(),
            identity_type: IdentityType::EmailAddress,
            verification_status: VerificationStatus::Success,
            verification_token: None,
            dkim_enabled: false,
            notification_topics: HashMap::new(),
            feedback_forwarding_enabled: true,
        };
        self.identities.insert(email.to_string(), record.clone());
        record
    }

    /// Add a domain identity. Auto-verifies in local dev mode.
    pub fn verify_domain(&self, domain: &str) -> IdentityRecord {
        let token = uuid::Uuid::new_v4().to_string();
        let record = IdentityRecord {
            identity: domain.to_string(),
            identity_type: IdentityType::Domain,
            verification_status: VerificationStatus::Success,
            verification_token: Some(token),
            dkim_enabled: false,
            notification_topics: HashMap::new(),
            feedback_forwarding_enabled: true,
        };
        self.identities.insert(domain.to_string(), record.clone());
        record
    }

    /// Check if an email address is verified (either directly or via domain).
    pub fn is_verified(&self, email: &str) -> bool {
        // Direct email match
        if let Some(record) = self.identities.get(email) {
            return matches!(record.verification_status, VerificationStatus::Success);
        }
        // Domain match: extract domain from email and check
        if let Some(domain) = email.split('@').nth(1) {
            if let Some(record) = self.identities.get(domain) {
                return matches!(record.verification_status, VerificationStatus::Success);
            }
        }
        false
    }

    /// List all identity strings, optionally filtered by type.
    pub fn list(&self, identity_type: Option<&IdentityType>) -> Vec<String> {
        self.identities
            .iter()
            .filter(|entry| {
                identity_type.map_or(true, |t| {
                    matches!(
                        (&entry.identity_type, t),
                        (IdentityType::EmailAddress, IdentityType::EmailAddress)
                            | (IdentityType::Domain, IdentityType::Domain)
                    )
                })
            })
            .map(|entry| entry.identity.clone())
            .collect()
    }

    /// Delete an identity by its string.
    pub fn delete(&self, identity: &str) {
        self.identities.remove(identity);
    }

    /// Get verification attributes for a list of identities.
    pub fn get_verification_attributes(
        &self,
        identities: &[String],
    ) -> HashMap<String, VerificationAttributes> {
        let mut result = HashMap::new();
        for identity in identities {
            let attrs = if let Some(record) = self.identities.get(identity) {
                VerificationAttributes {
                    verification_status: record.verification_status.clone(),
                    verification_token: record.verification_token.clone(),
                }
            } else {
                // Unknown identities still return Success in local dev
                VerificationAttributes {
                    verification_status: VerificationStatus::Success,
                    verification_token: if identity.contains('@') {
                        None
                    } else {
                        Some(uuid::Uuid::new_v4().to_string())
                    },
                }
            };
            result.insert(identity.clone(), attrs);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VerificationAttributes {
    pub verification_status: VerificationStatus,
    pub verification_token: Option<String>,
}
```

### 9.4 EmailStore (Retrospection)

This is the most important store -- it captures all "sent" emails for test retrospection.

```rust
/// Store for all sent emails, enabling retrospection via /_aws/ses.
///
/// Append-only: emails are added when SendEmail, SendRawEmail, or
/// SendTemplatedEmail is called. Emails can be queried by message ID
/// or source address, and cleared for test isolation.
pub struct EmailStore {
    /// All sent emails keyed by message ID.
    emails: DashMap<String, SentEmail>,
    /// Total number of emails sent (monotonically increasing, not reset on clear).
    total_sent: AtomicU64,
}

/// A single captured email, stored for retrospection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEmail {
    /// Unique message ID (UUID).
    pub id: String,
    /// AWS region where the email was sent.
    pub region: String,
    /// ISO 8601 timestamp of when the email was captured.
    pub timestamp: String,
    /// Source (From) email address.
    pub source: String,
    /// Destination addresses.
    pub destination: SentEmailDestination,
    /// Email subject line (for SendEmail, SendTemplatedEmail after rendering).
    pub subject: Option<String>,
    /// Email body.
    pub body: Option<SentEmailBody>,
    /// Raw MIME data (for SendRawEmail).
    pub raw_data: Option<String>,
    /// Template name (for SendTemplatedEmail).
    pub template: Option<String>,
    /// Template data JSON string (for SendTemplatedEmail).
    pub template_data: Option<String>,
    /// Message tags from the send request.
    pub tags: Vec<SentEmailTag>,
}

/// Destination addresses for a sent email.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEmailDestination {
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub bcc_addresses: Vec<String>,
}

/// Body of a sent email.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEmailBody {
    pub text_part: Option<String>,
    pub html_part: Option<String>,
}

/// A message tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEmailTag {
    pub name: String,
    pub value: String,
}

impl EmailStore {
    pub fn new() -> Self {
        Self {
            emails: DashMap::new(),
            total_sent: AtomicU64::new(0),
        }
    }

    /// Store a sent email for retrospection. Returns the generated message ID.
    pub fn capture(&self, email: SentEmail) -> String {
        let id = email.id.clone();
        self.emails.insert(id.clone(), email);
        self.total_sent.fetch_add(1, Ordering::Relaxed);
        id
    }

    /// Query emails with optional filters.
    pub fn query(
        &self,
        filter_id: Option<&String>,
        filter_source: Option<&String>,
    ) -> Vec<SentEmail> {
        self.emails
            .iter()
            .filter(|entry| {
                let email = entry.value();
                let id_match = filter_id
                    .map_or(true, |id| id.is_empty() || email.id == *id);
                let source_match = filter_source
                    .map_or(true, |src| src.is_empty() || email.source == *src);
                id_match && source_match
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Remove a specific email by message ID.
    pub fn remove(&self, id: &str) {
        self.emails.remove(id);
    }

    /// Clear all captured emails. Does NOT reset the total_sent counter.
    pub fn clear(&self) {
        self.emails.clear();
    }

    /// Get the total number of emails sent (lifetime, not reset on clear).
    pub fn total_sent(&self) -> u64 {
        self.total_sent.load(Ordering::Relaxed)
    }

    /// Get the current number of stored emails.
    pub fn count(&self) -> usize {
        self.emails.len()
    }
}
```

### 9.5 TemplateStore

```rust
/// Store for email templates.
///
/// Templates are keyed by name and contain subject, text body, and HTML body
/// with `{{variable}}` placeholders for substitution.
pub struct TemplateStore {
    templates: DashMap<String, EmailTemplate>,
}

/// An email template.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailTemplate {
    /// Template name (unique identifier).
    pub name: String,
    /// Subject line template with `{{variable}}` placeholders.
    pub subject_part: Option<String>,
    /// Plain text body template.
    pub text_part: Option<String>,
    /// HTML body template.
    pub html_part: Option<String>,
    /// Creation timestamp.
    pub created_timestamp: String,
}

impl TemplateStore {
    pub fn new() -> Self {
        Self {
            templates: DashMap::new(),
        }
    }

    pub fn create(&self, template: EmailTemplate) -> Result<(), SesError> {
        if self.templates.contains_key(&template.name) {
            return Err(SesError::already_exists(
                &format!("Template {} already exists.", template.name),
            ));
        }
        self.templates.insert(template.name.clone(), template);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<EmailTemplate, SesError> {
        self.templates
            .get(name)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SesError::template_does_not_exist(name))
    }

    pub fn update(&self, template: EmailTemplate) -> Result<(), SesError> {
        if !self.templates.contains_key(&template.name) {
            return Err(SesError::template_does_not_exist(&template.name));
        }
        self.templates.insert(template.name.clone(), template);
        Ok(())
    }

    pub fn delete(&self, name: &str) {
        self.templates.remove(name);
    }

    pub fn list(&self) -> Vec<TemplateMetadata> {
        self.templates
            .iter()
            .map(|entry| TemplateMetadata {
                name: entry.name.clone(),
                created_timestamp: entry.created_timestamp.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMetadata {
    pub name: String,
    pub created_timestamp: String,
}
```

### 9.6 Template Rendering

```rust
/// Render a template by substituting `{{variable}}` placeholders
/// with values from the template_data JSON.
///
/// Uses simple Mustache-style substitution. Does not support conditionals,
/// loops, or any advanced Handlebars features. This is sufficient for
/// the vast majority of SES template use cases.
pub fn render_template(
    template_text: &str,
    template_data: &str,
) -> Result<String, SesError> {
    let data: serde_json::Value = serde_json::from_str(template_data)
        .map_err(|e| SesError::invalid_template(
            &format!("Invalid template data JSON: {e}"),
        ))?;

    let data_map = data.as_object().ok_or_else(|| {
        SesError::invalid_template("Template data must be a JSON object")
    })?;

    let mut result = template_text.to_string();
    for (key, value) in data_map {
        let placeholder = format!("{{{{{key}}}}}");
        let replacement = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        result = result.replace(&placeholder, &replacement);
    }

    Ok(result)
}
```

### 9.7 ConfigurationSetStore

```rust
/// Store for configuration sets and their event destinations.
pub struct ConfigurationSetStore {
    /// Configuration sets keyed by name.
    config_sets: DashMap<String, ConfigurationSet>,
}

/// A configuration set with optional event destinations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationSet {
    pub name: String,
    pub event_destinations: Vec<EventDestination>,
}

/// An event destination within a configuration set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDestination {
    /// Event destination name.
    pub name: String,
    /// Whether the destination is enabled.
    pub enabled: bool,
    /// Matching event types (Send, Delivery, Bounce, etc.).
    pub matching_event_types: Vec<String>,
    /// SNS destination topic ARN (if SNS destination).
    pub sns_destination_topic_arn: Option<String>,
}

impl ConfigurationSetStore {
    pub fn new() -> Self {
        Self {
            config_sets: DashMap::new(),
        }
    }

    pub fn create(&self, name: &str) -> Result<(), SesError> {
        if self.config_sets.contains_key(name) {
            return Err(SesError::already_exists(
                &format!("Configuration set <{name}> already exists."),
            ));
        }
        self.config_sets.insert(
            name.to_string(),
            ConfigurationSet {
                name: name.to_string(),
                event_destinations: Vec::new(),
            },
        );
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), SesError> {
        self.config_sets
            .remove(name)
            .ok_or_else(|| SesError::configuration_set_does_not_exist(name))?;
        Ok(())
    }

    pub fn describe(&self, name: &str) -> Result<ConfigurationSet, SesError> {
        self.config_sets
            .get(name)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SesError::configuration_set_does_not_exist(name))
    }

    pub fn list(&self) -> Vec<String> {
        self.config_sets.iter().map(|e| e.key().clone()).collect()
    }

    pub fn add_event_destination(
        &self,
        config_set_name: &str,
        destination: EventDestination,
    ) -> Result<(), SesError> {
        let mut entry = self
            .config_sets
            .get_mut(config_set_name)
            .ok_or_else(|| SesError::configuration_set_does_not_exist(config_set_name))?;
        entry.event_destinations.push(destination);
        Ok(())
    }

    pub fn delete_event_destination(
        &self,
        config_set_name: &str,
        destination_name: &str,
    ) -> Result<(), SesError> {
        let mut entry = self
            .config_sets
            .get_mut(config_set_name)
            .ok_or_else(|| SesError::configuration_set_does_not_exist(config_set_name))?;
        entry
            .event_destinations
            .retain(|d| d.name != destination_name);
        Ok(())
    }

    /// Get event destinations for a configuration set (used during SendEmail
    /// to look up SNS notification targets).
    pub fn get_event_destinations(
        &self,
        config_set_name: &str,
    ) -> Option<Vec<EventDestination>> {
        self.config_sets
            .get(config_set_name)
            .map(|entry| entry.event_destinations.clone())
    }
}
```

### 9.8 ReceiptRuleSetStore

```rust
/// Store for receipt rule sets and rules.
///
/// Receipt rules are accepted and stored but never actually process
/// incoming email. This is for API compatibility only.
pub struct ReceiptRuleSetStore {
    rule_sets: DashMap<String, ReceiptRuleSet>,
    active_rule_set: std::sync::RwLock<Option<String>>,
}

/// A receipt rule set containing rules.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptRuleSet {
    pub name: String,
    pub rules: Vec<ReceiptRule>,
    pub created_timestamp: String,
}

/// A receipt rule within a rule set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptRule {
    pub name: String,
    pub enabled: bool,
    pub recipients: Vec<String>,
    pub actions: Vec<serde_json::Value>,
    pub scan_enabled: bool,
    pub tls_policy: String,
}

impl ReceiptRuleSetStore {
    pub fn new() -> Self {
        Self {
            rule_sets: DashMap::new(),
            active_rule_set: std::sync::RwLock::new(None),
        }
    }

    pub fn create_rule_set(&self, name: &str) -> Result<(), SesError> {
        if self.rule_sets.contains_key(name) {
            return Err(SesError::already_exists(
                &format!("Receipt rule set <{name}> already exists."),
            ));
        }
        self.rule_sets.insert(
            name.to_string(),
            ReceiptRuleSet {
                name: name.to_string(),
                rules: Vec::new(),
                created_timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
        Ok(())
    }

    pub fn delete_rule_set(&self, name: &str) -> Result<(), SesError> {
        self.rule_sets
            .remove(name)
            .ok_or_else(|| SesError::rule_set_does_not_exist(name))?;
        Ok(())
    }

    pub fn describe_rule_set(&self, name: &str) -> Result<ReceiptRuleSet, SesError> {
        self.rule_sets
            .get(name)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SesError::rule_set_does_not_exist(name))
    }

    pub fn create_rule(
        &self,
        rule_set_name: &str,
        rule: ReceiptRule,
        after: Option<&str>,
    ) -> Result<(), SesError> {
        let mut entry = self
            .rule_sets
            .get_mut(rule_set_name)
            .ok_or_else(|| SesError::rule_set_does_not_exist(rule_set_name))?;

        if let Some(after_name) = after {
            let pos = entry
                .rules
                .iter()
                .position(|r| r.name == after_name)
                .map(|p| p + 1)
                .unwrap_or(entry.rules.len());
            entry.rules.insert(pos, rule);
        } else {
            entry.rules.push(rule);
        }
        Ok(())
    }

    pub fn delete_rule(
        &self,
        rule_set_name: &str,
        rule_name: &str,
    ) -> Result<(), SesError> {
        let mut entry = self
            .rule_sets
            .get_mut(rule_set_name)
            .ok_or_else(|| SesError::rule_set_does_not_exist(rule_set_name))?;
        entry.rules.retain(|r| r.name != rule_name);
        Ok(())
    }

    pub fn clone_rule_set(
        &self,
        source_name: &str,
        dest_name: &str,
    ) -> Result<(), SesError> {
        let source = self.describe_rule_set(source_name)?;
        if self.rule_sets.contains_key(dest_name) {
            return Err(SesError::already_exists(
                &format!("Receipt rule set <{dest_name}> already exists."),
            ));
        }
        self.rule_sets.insert(
            dest_name.to_string(),
            ReceiptRuleSet {
                name: dest_name.to_string(),
                rules: source.rules.clone(),
                created_timestamp: chrono::Utc::now().to_rfc3339(),
            },
        );
        Ok(())
    }
}
```

### 9.9 SendStatistics

```rust
/// Tracks send statistics for GetSendStatistics and GetSendQuota.
pub struct SendStatistics {
    /// Number of successful sends.
    send_count: AtomicU64,
    /// Number of bounces (always 0 in local dev).
    bounce_count: AtomicU64,
    /// Number of complaints (always 0 in local dev).
    complaint_count: AtomicU64,
    /// Number of delivery attempts.
    delivery_attempts: AtomicU64,
    /// Number of rejects (always 0 in local dev).
    reject_count: AtomicU64,
}

impl SendStatistics {
    pub fn new() -> Self {
        Self {
            send_count: AtomicU64::new(0),
            bounce_count: AtomicU64::new(0),
            complaint_count: AtomicU64::new(0),
            delivery_attempts: AtomicU64::new(0),
            reject_count: AtomicU64::new(0),
        }
    }

    pub fn record_send(&self) {
        self.send_count.fetch_add(1, Ordering::Relaxed);
        self.delivery_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> SendStats {
        SendStats {
            send_count: self.send_count.load(Ordering::Relaxed),
            bounce_count: self.bounce_count.load(Ordering::Relaxed),
            complaint_count: self.complaint_count.load(Ordering::Relaxed),
            delivery_attempts: self.delivery_attempts.load(Ordering::Relaxed),
            reject_count: self.reject_count.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SendStats {
    pub send_count: u64,
    pub bounce_count: u64,
    pub complaint_count: u64,
    pub delivery_attempts: u64,
    pub reject_count: u64,
}
```

### 9.10 AccountState (SES v2)

```rust
/// Account-level state for SES v2 GetAccount/PutAccountDetails.
pub struct AccountState {
    /// Whether the account is in the SES sandbox (always false for local dev).
    pub production_access_enabled: bool,
    /// Account-level sending quota.
    pub send_quota: SendQuotaConfig,
    /// Account details.
    pub details: std::sync::RwLock<AccountDetails>,
}

#[derive(Debug, Clone)]
pub struct SendQuotaConfig {
    /// Max sends per 24 hours (default: unlimited for local dev).
    pub max_24_hour_send: f64,
    /// Max sends per second.
    pub max_send_rate: f64,
}

impl Default for SendQuotaConfig {
    fn default() -> Self {
        Self {
            max_24_hour_send: 200.0, // SES sandbox default
            max_send_rate: 1.0,      // SES sandbox default
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AccountDetails {
    pub mail_type: Option<String>,
    pub website_url: Option<String>,
    pub use_case_description: Option<String>,
    pub additional_contact_email_addresses: Vec<String>,
    pub contact_language: Option<String>,
}
```

### 9.11 Concurrency Model

Like SNS and SSM, SES has no real-time constraints, no streaming, and no complex transactions. `DashMap` provides sufficient concurrent access:

- **Reads** (GetSendQuota, ListIdentities, query emails, etc.): lock-free concurrent reads
- **Writes** (SendEmail captures, VerifyEmailIdentity, etc.): per-entry write locks via DashMap
- **Statistics counters**: `AtomicU64` for lock-free increment

No background processing is needed. All operations are synchronous request/response.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main SES provider implementing all operations.
pub struct RustackSes {
    pub(crate) state: Arc<SesState>,
    pub(crate) config: Arc<SesConfig>,
}

impl RustackSes {
    pub fn new(config: SesConfig) -> Self {
        Self {
            state: Arc::new(SesState {
                identities: IdentityStore::new(),
                emails: EmailStore::new(),
                templates: TemplateStore::new(),
                configuration_sets: ConfigurationSetStore::new(),
                receipt_rule_sets: ReceiptRuleSetStore::new(),
                statistics: SendStatistics::new(),
                account: AccountState::default(),
            }),
            config: Arc::new(config),
        }
    }
}
```

### 10.2 Operations by Phase

#### Phase 0: Core Sending + Identities (12 operations)

**VerifyEmailIdentity** -- Verify an email address identity.

1. Store the email address in IdentityStore with `VerificationStatus::Success`
2. In local dev, verification always succeeds immediately (no verification email sent)
3. Return empty response (success)

**VerifyDomainIdentity** -- Verify a domain identity.

1. Store the domain in IdentityStore with `VerificationStatus::Success`
2. Generate a random verification token (UUID)
3. Return `{ VerificationToken }` in XML response

**ListIdentities** -- List all verified identities.

1. Optionally filter by `IdentityType` (EmailAddress, Domain)
2. Support pagination with `MaxItems` (default 100) and `NextToken`
3. Return `{ Identities: [identity1, identity2, ...], NextToken }`

**DeleteIdentity** -- Delete a verified identity.

1. Remove the identity from IdentityStore
2. Return empty response (success). No error if identity does not exist.

**GetIdentityVerificationAttributes** -- Get verification status for identities.

1. For each identity in the request, return verification status
2. In local dev, all identities return `Success` (even unregistered ones)
3. For domain identities, include the verification token
4. Return `{ VerificationAttributes: { identity: { VerificationStatus, VerificationToken } } }`

**VerifyEmailAddress** -- Legacy API for verifying an email address.

1. Same as VerifyEmailIdentity but uses the older API shape
2. Return empty response

**DeleteVerifiedEmailAddress** -- Legacy API for deleting a verified email.

1. Same as DeleteIdentity for email addresses
2. Return empty response

**ListVerifiedEmailAddresses** -- Legacy API for listing verified emails.

1. Return only email identities (not domains) from IdentityStore
2. Return `{ VerifiedEmailAddresses: [email1, email2, ...] }`

**SendEmail** -- Send a formatted email.

1. Validate message tags if present:
   - Tag name: non-empty, max 255 chars, matches `[A-Za-z0-9_-]` (with `ses:` prefix exception)
   - Tag value: non-empty, max 255 chars, matches `[A-Za-z0-9_\-.@]`
2. Optionally validate source is verified (configurable via `SesConfig::require_verified_identity`)
3. Generate a unique message ID (UUID)
4. Capture the email in EmailStore for retrospection:
   ```rust
   let sent = SentEmail {
       id: message_id.clone(),
       region: context.region.clone(),
       timestamp: chrono::Utc::now().to_rfc3339(),
       source: input.source.clone(),
       destination: SentEmailDestination {
           to_addresses: destination.to_addresses.unwrap_or_default(),
           cc_addresses: destination.cc_addresses.unwrap_or_default(),
           bcc_addresses: destination.bcc_addresses.unwrap_or_default(),
       },
       subject: message.subject.as_ref().map(|s| s.data.clone()),
       body: Some(SentEmailBody {
           text_part: message.body.text.as_ref().map(|t| t.data.clone()),
           html_part: message.body.html.as_ref().map(|h| h.data.clone()),
       }),
       raw_data: None,
       template: None,
       template_data: None,
       tags: extract_tags(&input.tags),
   };
   self.state.emails.capture(sent);
   ```
5. Increment send statistics
6. Return `{ MessageId }` in XML response

**SendRawEmail** -- Send a raw MIME email.

1. Extract source from `Source` parameter or from `From:` header in raw MIME data
2. If source cannot be determined, return `MessageRejected`
3. Generate message ID
4. Capture in EmailStore with `raw_data` populated
5. Increment send statistics
6. Return `{ MessageId }`

**GetSendQuota** -- Get current send quota.

1. Return configurable quota values:
   ```xml
   <GetSendQuotaResult>
     <Max24HourSend>200.0</Max24HourSend>
     <MaxSendRate>1.0</MaxSendRate>
     <SentLast24Hours>{total_sent}</SentLast24Hours>
   </GetSendQuotaResult>
   ```

**GetSendStatistics** -- Get send statistics.

1. Return accumulated statistics as a single data point:
   ```xml
   <GetSendStatisticsResult>
     <SendDataPoints>
       <member>
         <DeliveryAttempts>{delivery_attempts}</DeliveryAttempts>
         <Bounces>{bounces}</Bounces>
         <Complaints>{complaints}</Complaints>
         <Rejects>{rejects}</Rejects>
         <Timestamp>{timestamp}</Timestamp>
       </member>
     </SendDataPoints>
   </GetSendStatisticsResult>
   ```

#### Phase 1: Templates + Configuration Sets (10 operations)

**CreateTemplate** -- Create an email template.

1. Validate template name is provided
2. Check template does not already exist
3. Store template with subject_part, text_part, html_part
4. Return empty response

**GetTemplate** -- Retrieve a template.

1. Look up template by name
2. Return `{ Template: { TemplateName, SubjectPart, TextPart, HtmlPart } }`
3. Error if not found: `TemplateDoesNotExist`

**UpdateTemplate** -- Update an existing template.

1. Verify template exists
2. Replace template content
3. Return empty response

**DeleteTemplate** -- Delete a template.

1. Remove template from store
2. Return empty response (no error if not found, matching AWS behavior)

**ListTemplates** -- List all templates.

1. Return template metadata (name, creation timestamp)
2. Support pagination with `MaxItems` and `NextToken`
3. Return `{ TemplatesMetadata: [{ Name, CreatedTimestamp }] }`

**SendTemplatedEmail** -- Send an email using a template.

1. Retrieve template by name (error if not found)
2. Parse `TemplateData` as JSON
3. Render template: substitute `{{variable}}` placeholders in subject, text body, and HTML body
4. Validate tags (same as SendEmail)
5. Capture rendered email in EmailStore (store both template reference and rendered content)
6. Increment send statistics
7. Return `{ MessageId }`

**CreateConfigurationSet** -- Create a configuration set.

1. Validate name
2. Check for duplicates
3. Store empty configuration set
4. Return empty response

**DeleteConfigurationSet** -- Delete a configuration set.

1. Remove configuration set and all its event destinations
2. Error if not found: `ConfigurationSetDoesNotExist`
3. Return empty response

**DescribeConfigurationSet** -- Describe a configuration set.

1. Return configuration set with its event destinations
2. Error if not found
3. Return `{ ConfigurationSet: { Name }, EventDestinations: [...] }`

**ListConfigurationSets** -- List all configuration sets.

1. Return list of configuration set names
2. Support pagination
3. Return `{ ConfigurationSets: [{ Name }] }`

#### Phase 2: Configuration Set Events + Receipt Rules (8 operations)

**CreateConfigurationSetEventDestination** -- Add an event destination.

1. Validate configuration set exists
2. Validate SNS destination topic ARN (if SNS destination, check topic exists in SNS service -- future enhancement)
3. Store event destination
4. Return empty response

**UpdateConfigurationSetEventDestination** -- Update an event destination.

1. Validate configuration set exists
2. Find and replace existing event destination by name
3. Return empty response

**DeleteConfigurationSetEventDestination** -- Remove an event destination.

1. Validate configuration set exists
2. Remove event destination by name
3. Return empty response

**CreateReceiptRuleSet** -- Create a receipt rule set.

1. Store empty rule set
2. Return empty response

**DeleteReceiptRuleSet** -- Delete a receipt rule set.

1. Remove rule set and all its rules
2. Return empty response

**CreateReceiptRule** -- Create a receipt rule in a rule set.

1. Validate rule set exists
2. Add rule (optionally after a specified rule)
3. Return empty response

**DeleteReceiptRule** -- Delete a receipt rule.

1. Remove rule from rule set
2. Return empty response

**DescribeReceiptRuleSet** -- Describe a receipt rule set.

1. Return rule set with all rules
2. Return `{ Metadata: { Name, CreatedTimestamp }, Rules: [...] }`

**CloneReceiptRuleSet** -- Clone a receipt rule set.

1. Copy all rules from source rule set to a new rule set
2. Error if source does not exist or destination already exists
3. Return empty response

#### Phase 3: SES v2 Core (10 operations)

SES v2 operations share the same underlying stores as v1. The v2 API provides a more modern interface with JSON bodies and path-based routing.

**CreateEmailIdentity (v2)** -- Create and verify an email identity.

1. Auto-verify (same as VerifyEmailIdentity/VerifyDomainIdentity)
2. Detect type: if contains `@`, it's EmailAddress; otherwise Domain
3. Return `{ IdentityType, VerifiedForSendingStatus: true }`

**GetEmailIdentity (v2)** -- Get an email identity.

1. Extract identity from path: `/v2/email/identities/{EmailIdentity}`
2. Return identity details including verification status
3. Return `{ IdentityType, VerifiedForSendingStatus, DkimAttributes, ... }`

**DeleteEmailIdentity (v2)** -- Delete an email identity.

1. Extract identity from path
2. Remove from IdentityStore
3. Return 200 empty

**ListEmailIdentities (v2)** -- List email identities.

1. Return all identities with type and sending status
2. Support pagination with `PageSize` and `NextToken`
3. Return `{ EmailIdentities: [{ IdentityType, IdentityName, SendingEnabled }] }`

**SendEmail (v2)** -- Send an email via v2 API.

1. Parse JSON body for `FromEmailAddress`, `Destination`, `Content`
2. `Content` can be `Simple` (subject + body) or `Raw` (raw MIME) or `Template`
3. Capture in EmailStore (same store as v1)
4. Return `{ MessageId }`

**CreateEmailTemplate (v2)** -- Create a template via v2 API.

1. Parse JSON body for `TemplateName`, `TemplateContent` (Subject, Text, Html)
2. Store in TemplateStore (same store as v1)
3. Return 200 empty

**GetEmailTemplate (v2)** -- Get a template via v2 API.

1. Extract template name from path
2. Return `{ TemplateName, TemplateContent: { Subject, Text, Html } }`

**GetAccount (v2)** -- Get account details.

1. Return account-level information:
   ```json
   {
     "ProductionAccessEnabled": false,
     "SendQuota": {
       "Max24HourSend": 200.0,
       "MaxSendRate": 1.0,
       "SentLast24Hours": 5.0
     },
     "SendingEnabled": true,
     "DedicatedIpAutoWarmupEnabled": false,
     "EnforcementStatus": "HEALTHY"
   }
   ```

**PutAccountDetails (v2)** -- Update account details.

1. Store mail type, website URL, use case description, contact info
2. Return 200 empty

### 10.3 Email Retrospection Endpoint

The `/_aws/ses` endpoint is the primary value of this implementation. It is a REST endpoint that provides access to all captured emails.

**GET /_aws/ses**

Returns all captured emails as JSON. Supports optional query parameters:

| Parameter | Description |
|-----------|-------------|
| `id` | Filter by exact message ID |
| `email` | Filter by source (From) email address |

Response format:
```json
{
  "messages": [
    {
      "id": "00000000-1111-2222-3333-444444444444",
      "region": "us-east-1",
      "timestamp": "2026-03-19T10:30:00Z",
      "source": "sender@example.com",
      "destination": {
        "toAddresses": ["recipient@example.com"],
        "ccAddresses": [],
        "bccAddresses": []
      },
      "subject": "Test Subject",
      "body": {
        "textPart": "Hello World",
        "htmlPart": "<p>Hello World</p>"
      },
      "rawData": null,
      "template": null,
      "templateData": null,
      "tags": [
        { "name": "campaign", "value": "welcome" }
      ]
    }
  ]
}
```

**DELETE /_aws/ses**

Clears all captured emails. Supports optional query parameter:

| Parameter | Description |
|-----------|-------------|
| `id` | Delete only the email with this message ID (if absent, clear all) |

Returns HTTP 204 No Content.

**Usage pattern in tests:**

```python
# Python test example
import boto3
import requests

ses = boto3.client('ses', endpoint_url='http://localhost:4566')

# Send email via SES API
ses.send_email(
    Source='sender@example.com',
    Destination={'ToAddresses': ['recipient@example.com']},
    Message={
        'Subject': {'Data': 'Password Reset'},
        'Body': {'Text': {'Data': 'Click here to reset your password: ...'}}
    }
)

# Query retrospection endpoint to verify
resp = requests.get('http://localhost:4566/_aws/ses')
messages = resp.json()['messages']
assert len(messages) == 1
assert messages[0]['subject'] == 'Password Reset'
assert 'recipient@example.com' in messages[0]['destination']['toAddresses']

# Clean up for next test
requests.delete('http://localhost:4566/_aws/ses')
```

### 10.4 Validation Rules

| Field | Rule |
|-------|------|
| Tag name | Non-empty, max 255 chars, `[A-Za-z0-9_-]` (with `ses:` prefix exception) |
| Tag value | Non-empty, max 255 chars, `[A-Za-z0-9_\-.@]` |
| Template name | Non-empty, alphanumeric with hyphens |
| Configuration set name | Non-empty, alphanumeric with hyphens and underscores |
| Email address format | Basic validation (contains `@`), not full RFC 5322 |
| Source address | Must be verified (configurable; disabled by default in local dev) |

### 10.5 Identity Verification Mode

The SES implementation supports two modes controlled by `SesConfig::require_verified_identity`:

| Mode | Behavior | Use Case |
|------|----------|----------|
| **Permissive** (default) | Any source address is accepted, even if not verified. GetIdentityVerificationAttributes returns `Success` for all identities. | Quick local dev, CI testing |
| **Strict** | Source address must be explicitly verified via VerifyEmailIdentity or VerifyDomainIdentity. Unverified sends return `MessageRejected`. | Testing identity verification workflows |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// SES error codes matching the AWS API.
#[derive(Debug, Clone)]
pub enum SesErrorCode {
    /// Email address not verified or message rejected.
    MessageRejected,
    /// Configuration set does not exist.
    ConfigurationSetDoesNotExist,
    /// Template does not exist.
    TemplateDoesNotExist,
    /// Resource already exists (template, config set, etc.).
    AlreadyExists,
    /// Invalid parameter value (bad tag, invalid email, etc.).
    InvalidParameterValue,
    /// Rule set does not exist.
    RuleSetDoesNotExist,
    /// Rule does not exist.
    RuleDoesNotExist,
    /// Limit exceeded.
    LimitExceeded,
    /// Invalid template (parse error, missing fields).
    InvalidTemplate,
    /// Event destination does not exist.
    EventDestinationDoesNotExist,
    /// Invalid SNS destination.
    InvalidSnsDestination,
    /// Unknown operation.
    InvalidAction,
    /// Account sending paused.
    AccountSendingPaused,
}
```

### 11.2 Error Mapping

```rust
impl SesError {
    /// Map to HTTP status code, error code string, and message.
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match &self.code {
            SesErrorCode::MessageRejected =>
                (400, "MessageRejected", self.message.clone()),
            SesErrorCode::ConfigurationSetDoesNotExist =>
                (400, "ConfigurationSetDoesNotExist", self.message.clone()),
            SesErrorCode::TemplateDoesNotExist =>
                (400, "TemplateDoesNotExist", self.message.clone()),
            SesErrorCode::AlreadyExists =>
                (400, "AlreadyExists", self.message.clone()),
            SesErrorCode::InvalidParameterValue =>
                (400, "InvalidParameterValue", self.message.clone()),
            SesErrorCode::RuleSetDoesNotExist =>
                (400, "RuleSetDoesNotExist", self.message.clone()),
            SesErrorCode::InvalidTemplate =>
                (400, "InvalidTemplate", self.message.clone()),
            SesErrorCode::InvalidAction =>
                (400, "InvalidAction", self.message.clone()),
            SesErrorCode::LimitExceeded =>
                (400, "LimitExceeded", self.message.clone()),
            _ => (400, "InternalError", self.message.clone()),
        }
    }
}
```

### 11.3 Error Response Format (awsQuery XML)

```xml
<ErrorResponse xmlns="http://ses.amazonaws.com/doc/2010-12-01/">
  <Error>
    <Type>Sender</Type>
    <Code>MessageRejected</Code>
    <Message>Email address is not verified. The following identities failed the check in region US-EAST-1: sender@example.com</Message>
  </Error>
  <RequestId>aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</RequestId>
</ErrorResponse>
```

### 11.4 Error Response Format (SES v2 JSON)

```json
{
  "__type": "MessageRejected",
  "message": "Email address is not verified."
}
```

---

## 12. Server Integration

### 12.1 Feature Gate

SES support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns", "lambda", "events", "logs", "kms", "kinesis", "secretsmanager", "ses"]
ses = ["dep:rustack-ses-core", "dep:rustack-ses-http"]
```

### 12.2 Gateway Registration

SES is registered in the gateway alongside other services. The `/_aws/ses` retrospection endpoint is also registered at the gateway level.

```rust
// In gateway setup
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

// X-Amz-Target-based services first
#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDbServiceRouter::new(dynamodb_handler)));

#[cfg(feature = "ssm")]
services.push(Box::new(SsmServiceRouter::new(ssm_handler)));

#[cfg(feature = "secretsmanager")]
services.push(Box::new(SecretsManagerServiceRouter::new(secretsmanager_handler)));

// ... other X-Amz-Target services ...

// SES must be registered BEFORE SNS since both use form-urlencoded POST.
// SES checks SigV4 service=email; if it doesn't match, falls through to SNS.
#[cfg(feature = "ses")]
services.push(Box::new(SesServiceRouter::new(ses_handler)));

#[cfg(feature = "sns")]
services.push(Box::new(SnsServiceRouter::new(sns_handler)));

// S3 is always last (catch-all)
#[cfg(feature = "s3")]
services.push(Box::new(S3ServiceRouter::new(s3_handler)));
```

**Important ordering note**: The SES v1 router must be registered *before* the SNS router. Both match on `Content-Type: x-www-form-urlencoded` POST requests. SES's `matches()` additionally checks the SigV4 `Credential` service name for `email`. If it doesn't match `email`, the request falls through to the SNS router. This is unambiguous because the SigV4 service name is always present in authenticated requests.

The SES v2 path prefix (`/v2/email/`) is unique and does not conflict with any other service.

### 12.3 Retrospection Endpoint Registration

The `/_aws/ses` endpoint is registered similarly to health checks -- intercepted at the gateway level before service routing:

```rust
impl Service<http::Request<Incoming>> for GatewayService {
    fn call(&self, req: http::Request<Incoming>) -> Self::Future {
        // Intercept health checks
        if is_health_check(req.method(), req.uri().path()) {
            // ...
        }

        // Intercept SES retrospection endpoint
        #[cfg(feature = "ses")]
        if req.uri().path() == "/_aws/ses" {
            let retrospection = Arc::clone(&self.ses_retrospection);
            return Box::pin(async move {
                match *req.method() {
                    Method::GET => Ok(retrospection.handle_get(&query_params).await),
                    Method::DELETE => Ok(retrospection.handle_delete(&query_params).await),
                    _ => Ok(method_not_allowed()),
                }
            });
        }

        // Route to services...
    }
}
```

### 12.4 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "ssm": "running",
        "sns": "running",
        "ses": "running",
        ...
    }
}
```

### 12.5 Configuration

```rust
/// SES service configuration.
pub struct SesConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
    /// Whether to require verified identities for sending.
    /// When false (default), any source address is accepted.
    /// When true, source must be verified via VerifyEmailIdentity/VerifyDomainIdentity.
    pub require_verified_identity: bool,
    /// Max sends per 24 hours (for GetSendQuota). Default: 200 (sandbox).
    pub max_24_hour_send: f64,
    /// Max send rate per second (for GetSendQuota). Default: 1.0 (sandbox).
    pub max_send_rate: f64,
}

impl SesConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("SES_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            require_verified_identity: env_bool("SES_REQUIRE_VERIFIED_IDENTITY", false),
            max_24_hour_send: env_f64("SES_MAX_24_HOUR_SEND", 200.0),
            max_send_rate: env_f64("SES_MAX_SEND_RATE", 1.0),
        }
    }
}
```

### 12.6 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `SES_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for SES |
| `SES_REQUIRE_VERIFIED_IDENTITY` | `false` | Require verified identity for sending |
| `SES_MAX_24_HOUR_SEND` | `200.0` | Send quota for GetSendQuota response |
| `SES_MAX_SEND_RATE` | `1.0` | Send rate for GetSendQuota response |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **IdentityStore**: verify email, verify domain, is_verified (direct email, domain fallback), delete, list by type, get verification attributes for known and unknown identities
- **EmailStore**: capture email, query by id, query by source, query with both filters, remove single, clear all, total_sent counter behavior (not reset on clear)
- **TemplateStore**: create, get, update, delete, list, duplicate name rejection
- **Template rendering**: simple substitution, nested values, missing variables (left as-is), null values, numeric values, empty data object, invalid JSON
- **ConfigurationSetStore**: create, delete, describe, list, add/delete event destinations
- **ReceiptRuleSetStore**: create, delete, describe, create rule with position, clone rule set
- **Tag validation**: valid tags, empty name, empty value, too-long name, invalid characters, `ses:` prefix exception
- **SigV4 service extraction**: parse `email` from Authorization header, handle malformed headers

### 13.2 Integration Tests with aws-sdk-rust

```rust
// tests/integration/ses_tests.rs
#[tokio::test]
#[ignore]
async fn test_should_send_email_and_retrieve_via_retrospection() {
    let ses = aws_sdk_ses::Client::new(&config);

    // Verify identity
    ses.verify_email_identity()
        .email_address("sender@example.com")
        .send().await.unwrap();

    // Send email
    ses.send_email()
        .source("sender@example.com")
        .destination(Destination::builder()
            .to_addresses("recipient@example.com")
            .build())
        .message(Message::builder()
            .subject(Content::builder().data("Test Subject").build())
            .body(Body::builder()
                .text(Content::builder().data("Hello").build())
                .build())
            .build())
        .send().await.unwrap();

    // Query retrospection endpoint
    let resp = reqwest::get("http://localhost:4566/_aws/ses")
        .await.unwrap()
        .json::<serde_json::Value>()
        .await.unwrap();

    let messages = resp["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["subject"], "Test Subject");
    assert_eq!(messages[0]["source"], "sender@example.com");
}

#[tokio::test]
#[ignore]
async fn test_should_send_raw_email() {
    // SendRawEmail with MIME data, verify via retrospection
}

#[tokio::test]
#[ignore]
async fn test_should_send_templated_email() {
    // CreateTemplate, SendTemplatedEmail, verify rendered content via retrospection
}

#[tokio::test]
#[ignore]
async fn test_should_manage_identities() {
    // VerifyEmailIdentity, VerifyDomainIdentity, ListIdentities, DeleteIdentity,
    // GetIdentityVerificationAttributes round-trip
}

#[tokio::test]
#[ignore]
async fn test_should_manage_templates() {
    // CreateTemplate, GetTemplate, UpdateTemplate, DeleteTemplate, ListTemplates
}

#[tokio::test]
#[ignore]
async fn test_should_manage_configuration_sets() {
    // CreateConfigurationSet, DescribeConfigurationSet, ListConfigurationSets,
    // DeleteConfigurationSet
}

#[tokio::test]
#[ignore]
async fn test_should_return_send_quota_and_statistics() {
    // GetSendQuota, GetSendStatistics before and after sending
}

#[tokio::test]
#[ignore]
async fn test_should_clear_retrospection_endpoint() {
    // Send emails, verify count, DELETE /_aws/ses, verify empty
}

#[tokio::test]
#[ignore]
async fn test_should_filter_retrospection_by_source() {
    // Send from multiple sources, filter by email param
}

#[tokio::test]
#[ignore]
async fn test_should_clone_receipt_rule_set() {
    // CreateReceiptRuleSet, CreateReceiptRule, CloneReceiptRuleSet,
    // DescribeReceiptRuleSet on clone
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Verify identity
aws ses verify-email-identity --email-address sender@example.com --endpoint-url http://localhost:4566

# List identities
aws ses list-identities --endpoint-url http://localhost:4566

# Send email
aws ses send-email \
  --from sender@example.com \
  --destination "ToAddresses=recipient@example.com" \
  --message "Subject={Data=Test},Body={Text={Data=Hello}}" \
  --endpoint-url http://localhost:4566

# Check retrospection
curl http://localhost:4566/_aws/ses | jq .

# Get send quota
aws ses get-send-quota --endpoint-url http://localhost:4566

# Template operations
aws ses create-template --template '{"TemplateName":"hello","SubjectPart":"Hi {{name}}","TextPart":"Hello {{name}}"}' --endpoint-url http://localhost:4566
aws ses get-template --template-name hello --endpoint-url http://localhost:4566
aws ses list-templates --endpoint-url http://localhost:4566

# Clear retrospection
curl -X DELETE http://localhost:4566/_aws/ses

# SES v2 (if Phase 3 implemented)
aws sesv2 create-email-identity --email-identity sender@example.com --endpoint-url http://localhost:4566
aws sesv2 get-account --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack SES Tests

**Location:** `vendors/localstack/tests/aws/services/ses/test_ses.py`
**Coverage:** Comprehensive test cases covering:
- SendEmail, SendRawEmail, SendTemplatedEmail
- Identity verification (email and domain)
- GetIdentityVerificationAttributes
- Template CRUD operations
- Configuration set management with event destinations and SNS notifications
- Receipt rule set operations including CloneReceiptRuleSet
- Tag validation on send operations
- Email retrospection endpoint (GET and DELETE)
- Error handling (MessageRejected, TemplateDoesNotExist, etc.)

**How to run:**
```makefile
test-ses-localstack:
	cd vendors/localstack && python -m pytest tests/aws/services/ses/ \
		-k "not sns_notification" \
		--endpoint-url=http://localhost:4566
```

#### 13.4.2 Terraform AWS Provider

**Source:** `hashicorp/terraform-provider-aws` (ses resources)
**Coverage:** Tests `aws_ses_email_identity`, `aws_ses_domain_identity`, `aws_ses_template`, `aws_ses_configuration_set`, `aws_ses_receipt_rule_set`.
**How to run:**
```bash
pip install terraform-local
tflocal init && tflocal apply
```

### 13.5 CI Integration

```yaml
# .github/workflows/ses-ci.yml
name: SES CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-ses-model
      - run: cargo test -p rustack-ses-core
      - run: cargo test -p rustack-ses-http

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
          aws ses verify-email-identity --email-address test@example.com \
            --endpoint-url http://localhost:4566
          aws ses send-email --from test@example.com \
            --destination "ToAddresses=recipient@example.com" \
            --message "Subject={Data=Test},Body={Text={Data=Hello}}" \
            --endpoint-url http://localhost:4566
          # Verify retrospection
          curl -s http://localhost:4566/_aws/ses | python3 -c "
          import sys, json
          msgs = json.load(sys.stdin)['messages']
          assert len(msgs) == 1, f'Expected 1 message, got {len(msgs)}'
          assert msgs[0]['subject'] == 'Test'
          print('SES smoke test passed')
          "
      - run: |
          # Python integration tests
          pip install boto3 pytest requests
          pytest tests/integration/ses/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: Core Sending + Identities (3-4 days)

**Goal:** SendEmail, SendRawEmail, identity management, retrospection endpoint. Enough for any application that sends email via SES to work against Rustack, and for tests to verify sent emails.

1. **Day 1: Smithy Model + Scaffolding**
   - Obtain SES v1 Smithy model (`ses-2010-12-01.json`)
   - Add `codegen/services/ses.toml`
   - Generate `rustack-ses-model` crate
   - Create `rustack-ses-core` and `rustack-ses-http` crate scaffolding
   - Implement `SesOperation` enum and awsQuery router (reuse SNS form-parsing pattern)

2. **Day 2: Storage + Core Sending**
   - Implement `IdentityStore`, `EmailStore`, `SendStatistics`
   - Implement `VerifyEmailIdentity`, `VerifyDomainIdentity`, `ListIdentities`, `DeleteIdentity`
   - Implement `GetIdentityVerificationAttributes`
   - Implement `SendEmail` with tag validation and email capture
   - Implement `SendRawEmail` with MIME source extraction

3. **Day 3: Remaining Core + Gateway + Retrospection**
   - Implement `GetSendQuota`, `GetSendStatistics`
   - Implement legacy operations: `VerifyEmailAddress`, `DeleteVerifiedEmailAddress`, `ListVerifiedEmailAddresses`
   - Implement `/_aws/ses` retrospection endpoint (GET + DELETE)
   - Integrate SES v1 router into gateway (SigV4 service=`email` check)
   - Register retrospection endpoint in gateway

4. **Day 4: Tests + Polish**
   - Unit tests for all storage operations and template rendering
   - Integration tests with aws-sdk-rust (SendEmail + retrospection round-trip)
   - AWS CLI smoke tests
   - Fix edge cases from LocalStack test suite

**Deliverable:** AWS CLI `ses send-email` works, any AWS SDK can send emails, tests can verify emails via `/_aws/ses`.

### Phase 1: Templates + Configuration Sets (2 days)

**Goal:** Template management and SendTemplatedEmail. Configuration set CRUD.

5. **Day 5: Templates**
   - Implement `TemplateStore` and template rendering engine
   - Implement `CreateTemplate`, `GetTemplate`, `UpdateTemplate`, `DeleteTemplate`, `ListTemplates`
   - Implement `SendTemplatedEmail` with template resolution and `{{variable}}` substitution

6. **Day 6: Configuration Sets**
   - Implement `ConfigurationSetStore`
   - Implement `CreateConfigurationSet`, `DeleteConfigurationSet`, `DescribeConfigurationSet`, `ListConfigurationSets`

**Deliverable:** Terraform `aws_ses_template`, `aws_ses_configuration_set` work. SendTemplatedEmail with variable substitution works.

### Phase 2: Event Destinations + Receipt Rules (2 days)

**Goal:** Configuration set event destinations and receipt rule sets.

7. **Day 7: Event Destinations + Receipt Rules**
   - Implement `CreateConfigurationSetEventDestination`, `UpdateConfigurationSetEventDestination`, `DeleteConfigurationSetEventDestination`
   - Implement `ReceiptRuleSetStore`
   - Implement `CreateReceiptRuleSet`, `DeleteReceiptRuleSet`, `CreateReceiptRule`, `DeleteReceiptRule`, `DescribeReceiptRuleSet`, `CloneReceiptRuleSet`
   - Implement `DescribeActiveReceiptRuleSet`, `SetActiveReceiptRuleSet`

**Deliverable:** Full SES v1 API surface. LocalStack test suite (non-SNS-notification tests) passes.

### Phase 3: SES v2 Core (2 days)

**Goal:** Core SES v2 operations for tools that use the v2 API.

8. **Day 8: SES v2 Identity + Send**
   - Implement SES v2 HTTP router (path-based, restJson1)
   - Implement `CreateEmailIdentity`, `GetEmailIdentity`, `DeleteEmailIdentity`, `ListEmailIdentities` (v2)
   - Implement `SendEmail` (v2) with Simple/Raw/Template content types

9. **Day 9: SES v2 Templates + Account + CI**
   - Implement `CreateEmailTemplate`, `GetEmailTemplate` (v2)
   - Implement `GetAccount`, `PutAccountDetails` (v2)
   - CI workflow for SES
   - Update Docker image, GitHub Action, README
   - Run LocalStack test suite, document pass/fail

**Deliverable:** All 4 phases complete. SES v1 + v2 core operations. CI green.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| SigV4 service name parsing is fragile | Medium | High | The `Authorization` header format is well-defined by AWS SigV4 spec. Parse defensively; if service name cannot be extracted, fall through to SNS (existing behavior). Add comprehensive unit tests for header parsing edge cases (missing header, malformed credential, pre-signed URLs). |
| awsQuery form parameter encoding differs from SNS | Low | Medium | SES v1 uses the same `member.N.Key/member.N.Value` encoding as SNS. Reuse the same form parser. Test with real AWS SDK requests to verify encoding correctness. |
| Template rendering edge cases | Medium | Low | The `{{variable}}` substitution is intentionally simple. Document that we do not support Handlebars features (conditionals, loops, partials). If a variable is not in template_data, leave the placeholder as-is (matching basic Mustache behavior). |
| SES v2 path routing conflicts with other services | Low | Medium | The `/v2/email/` prefix is unique to SES v2. No other service uses this path prefix. Register the SES v2 matcher before the S3 catch-all. |
| SNS event destination integration | Medium | Medium | For MVP, event destinations are stored but events are not actually emitted to SNS topics. Future: call into `rustack-sns-core` to publish events. This is a non-blocking enhancement. |
| Large raw email bodies | Low | Low | MIME emails can be large. Store in memory as-is. For local dev, this is acceptable. Consider adding a configurable max email size limit if memory becomes a concern. |
| AWS SDK sends SES requests without SigV4 in some configurations | Low | High | Some test setups use unsigned requests. The `matches()` method should handle the case where no `Authorization` header is present. In this case, fall through to SNS (or add a secondary check based on `Action=` parameter names unique to SES). |

### 15.2 Fallback Routing Strategy

If the SigV4 service name approach proves too fragile (e.g., unsigned requests, pre-signed URLs), an alternative routing strategy is to buffer the form body and inspect the `Action=` parameter:

- SES-only actions: `SendEmail`, `SendRawEmail`, `VerifyEmailIdentity`, `VerifyDomainIdentity`, `GetSendQuota`, `CreateTemplate`, etc.
- SNS-only actions: `CreateTopic`, `Publish`, `Subscribe`, etc.
- There is zero overlap between SES and SNS action names.

This is the same approach used to distinguish SNS from SQS (both awsQuery). The tradeoff is that it requires buffering and parsing the request body before routing, which the gateway already does for SNS/SQS disambiguation.

```rust
/// Fallback SES routing: check if Action= is a known SES operation.
fn is_ses_action(form_body: &str) -> bool {
    if let Some(action) = extract_form_action(form_body) {
        SesOperation::from_name(&action).is_some()
    } else {
        false
    }
}
```

### 15.3 Dependencies

- `rustack-core` -- no changes needed
- `rustack-auth` -- no changes needed (SigV4 with service=`email` for v1, service=`ses` for v2)
- `dashmap` -- already in workspace
- `uuid` -- for message ID generation (already in workspace)
- `chrono` -- for timestamp formatting (already in workspace)
- `quick-xml` -- for XML response serialization (already in workspace from SNS)
- `serde_urlencoded` -- for form-urlencoded parsing (already in workspace from SNS)
- `serde_json` -- for template data parsing and retrospection API (already in workspace)

### 15.4 Decision Log

| Decision | Rationale |
|----------|-----------|
| SES v1 as primary, v2 as stretch goal | Many tools (Terraform, older SDKs, Django SES backend) still use v1. v1 is simpler (awsQuery, reuses SNS infra). v2 has 100+ operations but most are management APIs. |
| SigV4 service name for routing | The only reliable way to distinguish SES v1 from SNS -- both use identical Content-Type and request shape. The SigV4 `Credential` field always includes the service name. |
| Action-name fallback routing | As a safety net for unsigned requests, check `Action=` parameter names. SES and SNS have completely disjoint action names. |
| Auto-verify all identities by default | Local dev should not require explicit verification. Most tests just need SendEmail to work. Configurable via `SES_REQUIRE_VERIFIED_IDENTITY=true` for testing verification workflows. |
| Simple `{{variable}}` template rendering | Full Handlebars/Mustache is overkill. SES templates in the real service use a limited subset. Simple replacement covers 95%+ of use cases. |
| Email retrospection as REST endpoint (not SQS/SNS) | REST endpoint is the simplest integration for tests. Any HTTP client can query it. No dependency on other Rustack services. Matches LocalStack's approach. |
| Store raw MIME data as-is for SendRawEmail | Parsing MIME is complex and error-prone. Store the raw data and let tests inspect it directly. Only extract the `From:` header for the `source` field. |
| No actual email delivery, ever | This is a fundamental design principle. Rustack SES is a capture-and-inspect tool. Adding real SMTP delivery would be a security risk and a different product entirely. |
| SES v2 shares stores with v1 | v1 and v2 operate on the same underlying resources (identities, templates). A single set of stores ensures consistency regardless of which API version the client uses. |
