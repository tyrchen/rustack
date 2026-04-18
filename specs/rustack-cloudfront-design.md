# Rustack CloudFront: Native Rust Implementation Design

**Date:** 2026-04-18
**Status:** Draft / RFC
**Depends on:** [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md), [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md)
**Companion spec:** [rustack-cloudfront-dataplane-design.md](./rustack-cloudfront-dataplane-design.md) -- defines the minimal axum-based pass-through data plane that pairs with this management plane. The two specs are delivered in parallel tracks: management first (Phase 0-4), data plane phased D0-D2 layered on top.
**Scope:** Add Amazon CloudFront support to Rustack -- a management API for Distributions, Invalidations, Origin Access Controls, Origin Access Identities, Cache/Origin Request/Response Headers Policies, Key Groups, Public Keys, Realtime Log Configs, CloudFront Functions, Tags, and related resources. Protocol is `restXml` (identical family to S3). ~90 operations across 5 phases, reusing the existing Smithy-based codegen and `rustack-s3-xml` XML stack. A minimal CDN data plane (pass-through reverse proxy, no caching) is defined in the companion spec.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Why LocalStack Is Not a Reference](#4-why-localstack-is-not-a-reference)
5. [Architecture Overview](#5-architecture-overview)
6. [Protocol Design: restXml](#6-protocol-design-restxml)
7. [Smithy Code Generation Strategy](#7-smithy-code-generation-strategy)
8. [Crate Structure](#8-crate-structure)
9. [HTTP Layer Design](#9-http-layer-design)
10. [Storage Engine Design](#10-storage-engine-design)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [ETag / IfMatch Concurrency Model](#13-etag--ifmatch-concurrency-model)
14. [Distribution Status State Machine](#14-distribution-status-state-machine)
15. [Cross-Service Interactions](#15-cross-service-interactions)
16. [Server Integration](#16-server-integration)
17. [Testing Strategy](#17-testing-strategy)
18. [Phased Implementation Plan](#18-phased-implementation-plan)
19. [Risk Analysis](#19-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding CloudFront support to Rustack. Key design decisions:

- **Management plane here, minimal data plane in companion spec** -- this spec covers the CloudFront control API (create/get/update/delete/list Distributions, Invalidations, Origin Access Controls, etc.). A **companion spec** [rustack-cloudfront-dataplane-design.md](./rustack-cloudfront-dataplane-design.md) adds a minimal pass-through reverse proxy so that `curl http://localhost:4566/_aws/cloudfront/{id}/path` (or `curl http://{id}.cloudfront.net/path` with local DNS setup) returns content from the configured origin. The data plane does **no caching**, **no Lambda@Edge execution**, and **no function execution** -- it exists for end-to-end IaC testing, not CDN simulation.
- **restXml protocol, reused from S3** -- CloudFront and S3 share the restXml protocol family. The XML serialization and deserialization kernels in `rustack-s3-xml` (see `crates/rustack-s3-xml/src/serialize.rs` and `deserialize.rs`) are extracted into a generic `rustack-restxml` crate and reused. The Smithy codegen gets restXml parity for a second service.
- **Path-based routing, `/2020-05-31/` prefix** -- all CloudFront operations route under `/2020-05-31/distribution`, `/2020-05-31/invalidation`, `/2020-05-31/origin-access-control`, `/2020-05-31/cache-policy`, `/2020-05-31/key-group`, etc. The gateway matches this prefix or the SigV4 `cloudfront` service name.
- **Global service semantics** -- CloudFront is a global (us-east-1-only) service, similar to IAM. ARN formatting uses no region. The provider ignores region from the request and always uses the configured global region (`us-east-1` by default).
- **ETag / IfMatch concurrency** -- every config-bearing resource response carries an `ETag` header; mutating operations require `If-Match` matching the current ETag. Rustack implements ETags as monotonically increasing version tokens per resource and enforces `PreconditionFailed` (412) on mismatch. This is load-bearing -- Terraform and the AWS CLI rely on it to detect concurrent drift.
- **State machine on distribution lifecycle** -- real CloudFront distributions take ~15 minutes to propagate (`InProgress` → `Deployed`). Rustack fast-forwards: after `CreateDistribution` / `UpdateDistribution`, a background tokio task flips status to `Deployed` after a configurable delay (default 0 ms, but CI can simulate propagation by setting `CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS=30000`).
- **Invalidation state machine** -- `InProgress` → `Completed`, also fast-forwarded; invalidation history is retained so tests can assert invalidation paths.
- **In-memory storage** -- `DashMap`-backed, consistent with every other Rustack service. Data does not persist across restarts.
- **No Lambda@Edge or CloudFront Functions runtime** -- function definitions are stored as opaque code blobs; `TestFunction` returns a canned "success" result. Lambda@Edge associations are stored on distributions but never invoked.
- **No WAF, no Shield, no Signed URL verification** -- signed URL key material is stored but signatures are not verified because Rustack is not serving cached traffic.
- **Phased delivery** -- 5 phases from Phase 0 (Distribution + Invalidation + OAC + Tags, the 80% use case) through Phase 4 (advanced/edge features).

---

## 2. Motivation

### 2.1 Why CloudFront?

CloudFront is the CDN layer of virtually every non-trivial AWS static-site and API-cache deployment. Almost every production stack that serves web content at scale includes it. Consequently it shows up in the IaC and SDK code that developers want to run locally:

- **Static sites** -- S3 + CloudFront is the canonical pattern. Terraform/CDK templates for this pattern depend on `aws_cloudfront_distribution`, `aws_cloudfront_origin_access_control` (OAC, the modern replacement for OAI), and S3 bucket policies that grant OAC access. None of this works today against Rustack because `aws_cloudfront_*` requests 404.
- **API caching** -- API Gateway + CloudFront is used to add edge caching to REST/HTTP APIs.
- **Cache invalidation automation** -- CI pipelines routinely call `aws cloudfront create-invalidation --paths "/*"` after a deploy. Developers who integration-test their deploy scripts locally need this to succeed.
- **Signed URLs / signed cookies** -- many applications use CloudFront Key Groups + Public Keys + trusted signers to gate private video/PDF delivery. The management surface (create/rotate keys, associate with distributions) must work.
- **Terraform coverage** -- `aws_cloudfront_distribution` is one of the ten most-used AWS Terraform resources (per the public registry download numbers). Rustack's Terraform story is materially incomplete without it.
- **CDK `CloudFrontWebDistribution` / `Distribution`** -- standard CDK constructs for frontend deployments. `cdk deploy --profile rustack` fails as soon as it tries to create a distribution.

### 2.2 Value Proposition

| Benefit | Who it unblocks |
|---------|-----------------|
| `terraform apply` with `aws_cloudfront_*` resources | Platform engineers, DevOps, IaC authors |
| `cdk deploy` with CloudFront constructs | Frontend teams using CDK |
| `aws cloudfront create-invalidation` in CI | Deploy pipelines that bust CDN cache |
| boto3 / aws-sdk-rust tests against CloudFront | SDK-level application code |
| Realistic IaC drift detection testing | Platform teams debugging drift |

### 2.3 Complexity Assessment

| Dimension | CloudFront | S3 (for scale) | API Gateway v2 |
|-----------|-----------|----------------|----------------|
| Total operations | ~90 | ~70 | ~53 |
| Protocol | restXml | restXml | restJson1 |
| ETag / IfMatch semantics | Pervasive | Per-object | None |
| Distinct top-level resource kinds | 15+ | 2 (bucket, object) | 12 |
| Lifecycle state machine | Yes (Distribution, Invalidation) | Minimal | Yes (Deployment) |
| Cross-service deps | S3 bucket policy (optional), Lambda@Edge (stubbed) | None | Lambda |
| Est. lines of code | ~10,000-13,000 | 20,000+ | ~8,000-10,000 |

CloudFront is moderately large but conceptually regular: most resources follow a `Create/Get/GetConfig/Update/Delete/List` six-verb pattern with ETag/IfMatch semantics. Once the pattern is implemented for `Distribution`, the rest of the resource kinds fall out as repetition.

### 2.4 Tool Coverage

| Tool | Operations Used | Phase Available |
|------|-----------------|-----------------|
| AWS CLI (`aws cloudfront`) | All ops | Incremental by phase |
| Terraform (`aws_cloudfront_distribution`, `aws_cloudfront_origin_access_control`) | Distribution, OAC, Tags CRUD | Phase 0 |
| Terraform (`aws_cloudfront_origin_access_identity`, `aws_cloudfront_cache_policy`) | OAI, CachePolicy | Phase 1 |
| Terraform (`aws_cloudfront_key_group`, `aws_cloudfront_public_key`) | KeyGroup, PublicKey | Phase 2 |
| CDK `Distribution`, `OriginAccessControl`, `CachePolicy` | Same as Terraform | Phases 0-1 |
| boto3 / aws-sdk-rust | All | All phases |
| `aws cloudfront create-invalidation` | CreateInvalidation, GetInvalidation | Phase 0 |
| `curl`/browser/Playwright GET of distribution URL (via data-plane spec) | Data plane | Data-plane Phase D0 (S3 origin) / D1 (HTTP origin) / D2 (APIGW+Lambda URL) |
| End-to-end IaC testing (apply → curl → assert content) | Management + data plane | Data-plane Phase D0+ |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full CloudFront management API** -- CRUD for Distributions, Invalidations, Origin Access Controls, Origin Access Identities, Cache Policies, Origin Request Policies, Response Headers Policies, Key Groups, Public Keys, Field-Level Encryption configs/profiles, Realtime Log Configs, Tags (~90 operations across phases).
2. **restXml protocol parity with AWS** -- wire-compatible XML request/response formats, including nested config wrappers (`DistributionConfig`, `CachePolicyConfig`, etc.).
3. **ETag / IfMatch enforcement** -- every mutating operation on a versioned resource validates `If-Match` and returns `PreconditionFailed` on mismatch. Every read returns `ETag`.
4. **Distribution lifecycle simulation** -- `Status = InProgress` at creation/update, transitions to `Deployed` after a configurable (default zero) delay.
5. **Invalidation lifecycle simulation** -- `Status = InProgress` → `Completed`, with invalidation history preserved for inspection.
6. **Tags** -- uniform tagging across Distributions and other taggable resources.
7. **CloudFront Functions management** -- Create/Delete/Describe/List/Publish/Update; `TestFunction` returns a canned success payload (no JS runtime).
8. **Lambda@Edge association storage** -- distributions can reference Lambda function versions as edge triggers; stored but never invoked.
9. **Global service behavior** -- ARNs exclude region, service endpoint accepts `us-east-1` SigV4 signatures regardless of the request region.
10. **Smithy-generated model crate** -- all input/output types generated from the official `cloudfront.json` Smithy model via the universal codegen.
11. **Gateway routing** -- match `/2020-05-31/` path prefix OR SigV4 signing service `cloudfront`. Register before the S3 catch-all.
12. **Resource Policy + GetManagedCertificateDetails stubs** -- minimal stubs for operations that IaC tools occasionally emit but do not depend on semantically.
13. **Reproducible IDs** -- when `CLOUDFRONT_DETERMINISTIC_IDS=true`, resource IDs are derived from a hash of input so that snapshot tests are stable.

### 3.2 Non-Goals (for the management plane)

Note: non-goals 1-6 below address the management plane itself. Some of them are partially addressed by the companion data-plane spec (see [rustack-cloudfront-dataplane-design.md](./rustack-cloudfront-dataplane-design.md) for the full non-goal list on the data-plane side, which remains strict about caching, Lambda@Edge, and function execution).

1. **Full CDN data plane with caching is still a non-goal** -- the companion data-plane spec defines a **pass-through reverse proxy only**: no TTL evaluation, no cache storage, no stale-while-revalidate, no request coalescing. Users needing a real CDN run against AWS. The data plane exists solely for end-to-end IaC configuration testing.
2. **No cache semantics** -- cache policies, TTLs, `min_ttl`/`max_ttl`/`default_ttl`, origin cache headers, cache keys -- all stored and used for *reference resolution* (the data plane picks a behavior by policy ID), but their caching *semantics* are never enforced.
3. **No Lambda@Edge invocation** -- function ARNs are stored on distribution cache behaviors; the data plane logs a warning (or hard-fails under `CLOUDFRONT_FAIL_ON_FUNCTION=true`) when a request would have triggered one.
4. **No CloudFront Functions runtime** -- no JavaScript engine. `TestFunction` returns a canned successful payload. The data plane skips function associations with a warning (same failure switch as Lambda@Edge).
5. **No signed URL / signed cookie verification** -- public keys and key groups are stored, but signatures are never validated. The data plane logs a warning on distributions that reference `TrustedKeyGroups` / `TrustedSigners` and serves requests anyway.
6. **No real edge location simulation** -- no PoP selection, no geographic routing, no Edge IP allocation.
7. **No CloudFront Functions KeyValueStore persistence semantics** -- KV store records are stored but read/write semantics from within functions are not modeled (no runtime).
8. **No continuous deployment policy traffic shifting** -- policies stored but do not split traffic (there is no traffic).
9. **No Shield / WAF integration** -- WebACL associations are stored on distributions as opaque ARNs; WAF itself is out of scope for Rustack overall.
10. **No DNS validation for alternate domain names (CNAMEs)** -- `aliases` stored verbatim; ownership not checked via `ListConflictingAliases` / `VerifyDnsConfiguration` (these return canned successes).
11. **No Origin Shield behavior** -- configured but not enforced.
12. **No streaming distribution data plane (RTMP)** -- RTMP has been discontinued by AWS; we implement the management API shells only because Terraform still emits them.
13. **No `AssociateAlias` cross-account ownership checks** -- we simply update the distribution's aliases list.
14. **No persistence across Rustack restarts** -- consistent with other services.

---

## 4. Why LocalStack Is Not a Reference

CloudFront is a LocalStack Pro–only service. The submodule at `vendors/localstack/` (the open-source `localstack/localstack` repo) contains **no** CloudFront provider. Concretely, `ls vendors/localstack/localstack-core/localstack/services/` has no `cloudfront/` directory, and `grep -ri cloudfront vendors/localstack/localstack-core/` returns only incidental references in tag serializer utilities and test fixtures.

Consequently, Rustack's CloudFront support is a **clean-room implementation** based on:

1. The official AWS Smithy model (`cloudfront.json`) from `aws/aws-sdk-js-v3/codegen/sdk-codegen/aws-models/cloudfront.json`.
2. Public AWS documentation for semantics (ETag model, state machine, error codes).
3. Behavioral expectations encoded in the aws-sdk-rust integration tests and Terraform acceptance tests.

This is actually easier than a LocalStack port: the Smithy model is authoritative, and Rustack's codegen consumes it directly. We do not inherit LocalStack's moto-backed shims or their Pro-source obfuscation.

---

## 5. Architecture Overview

### 5.1 Single-Component Architecture

Unlike API Gateway v2 (which has management + execution), CloudFront is management-only. There is exactly one HTTP surface: the REST XML management API.

```
                AWS SDK / CLI / Terraform / CDK
                         |
                         | HTTP :4566
                         v
              +---------------------+
              |   Gateway Router    |
              +--------+------------+
                       |
              (path starts with /2020-05-31/
               OR SigV4 service=cloudfront)
                       |
                       v
              +---------------------+
              |   CloudFront HTTP   |  restXml
              |   (rustack-cloudfront-http)
              +---------+-----------+
                        |
                        v
              +---------------------+
              |   CloudFront Core   |  RustackCloudFront
              |   (rustack-cloudfront-core)
              +---------+-----------+
                        |
              +---------+-----------+
              |       Storage       |  DashMaps
              | distributions       |
              | invalidations       |
              | origin_access_ctl   |
              | cache_policies      |
              | key_groups          |
              | public_keys         |
              | functions           |
              | realtime_log_cfgs   |
              | tags                |
              +---------+-----------+
                        |
            optional    |
                        v
              +---------------------+
              |  S3 Provider (opt)  |  For OAC → bucket policy wire-up
              +---------------------+
```

### 5.2 Gateway Routing

CloudFront registers a single service router. The matching logic is:

```rust
fn matches(&self, req: &http::Request<Incoming>) -> bool {
    // Primary: path prefix
    if req.uri().path().starts_with("/2020-05-31/") {
        return true;
    }
    // Secondary: SigV4 service name (catches unusual paths)
    extract_sigv4_service(req.headers()).is_some_and(|svc| svc == "cloudfront")
}
```

**Registration order** (additions shown with `<-- NEW`):

1. Path-prefix services: `/v2/` (API Gateway v2), `/_aws/execute-api/` (API Gateway v2 execution), `/YYYY-MM-DD/functions*` (Lambda), `/2020-05-31/*` (CloudFront) `<-- NEW`
2. Header-based services: `X-Amz-Target: DynamoDB_*`, `AmazonSQS*`, `AmazonSSM.*`, `secretsmanager.*`, `TrentService.*`, `Kinesis_20131202.*`, `AWSEvents.*`, `Logs_20140328.*`, `GraniteServiceVersion20100801.*`
3. SigV4-discriminated form POST: SNS, IAM, STS, SES, CloudWatch awsQuery, CloudFront `<-- NEW` (fallback only)
4. Default: route to S3 (catch-all)

The `/2020-05-31/` prefix is unambiguous. No other AWS service shares this exact version-dated prefix. S3 bucket names cannot start with a digit followed by four more digits followed by a dash pattern, so there is no collision risk.

### 5.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-cloudfront-model       <-- NEW (auto-generated)
+-- rustack-cloudfront-http        <-- NEW
+-- rustack-cloudfront-core        <-- NEW
+-- rustack-restxml                <-- NEW (extracted from rustack-s3-xml)
+-- rustack-s3-{model,core,http}
+-- ... (other services)

rustack-cloudfront-http
+-- rustack-cloudfront-model
+-- rustack-restxml
+-- rustack-auth

rustack-cloudfront-core
+-- rustack-core
+-- rustack-cloudfront-model
+-- rustack-s3-core (optional, behind `s3-integration` feature, for OAC policy write-through)
+-- dashmap, tokio, uuid, chrono, tracing
```

`rustack-restxml` is carved out of `rustack-s3-xml` as part of this work. S3 and CloudFront both depend on it.

---

## 6. Protocol Design: restXml

### 6.1 Protocol Characteristics

CloudFront uses the Smithy `restXml` protocol, identical family to S3 but with its own conventions:

| Aspect | S3 restXml | CloudFront restXml |
|--------|-----------|--------------------|
| HTTP methods | GET, PUT, POST, DELETE, HEAD | GET, POST, PUT, DELETE |
| URL path | `/{bucket}/{key}` | `/2020-05-31/{resource-kind}/{id}` |
| Operation dispatch | HTTP method + path + sub-resource query params | HTTP method + path |
| Request body | Raw bytes (PutObject) or XML config | Always XML config (or empty) |
| Response body | Raw bytes or XML | Always XML |
| Content-Type (request) | Any | `application/xml` |
| ETag header | Per-object | Per-config-resource, mandatory |
| If-Match header | Conditional writes | **Required for Update/Delete** |
| Error format | `<Error><Code>...</Code></Error>` | `<ErrorResponse><Error><Code>...</Code></Error></ErrorResponse>` |
| XML root element | `<OperationNameResult>` or typed | Typed (e.g., `<Distribution>`, `<CreateDistributionResult>`) |
| XML namespace | `http://s3.amazonaws.com/doc/2006-03-01/` | `http://cloudfront.amazonaws.com/doc/2020-05-31/` |
| Default region | `us-east-1` | `us-east-1` (global service) |
| SigV4 service name | `s3` | `cloudfront` |

### 6.2 Request Anatomy

A typical `CreateDistribution` request:

```http
POST /2020-05-31/distribution HTTP/1.1
Content-Type: application/xml
Authorization: AWS4-HMAC-SHA256 Credential=.../cloudfront/aws4_request, ...

<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <CallerReference>terraform-20260418-01</CallerReference>
  <Aliases><Quantity>0</Quantity></Aliases>
  <DefaultRootObject>index.html</DefaultRootObject>
  <Origins>
    <Quantity>1</Quantity>
    <Items>
      <Origin>
        <Id>primary</Id>
        <DomainName>my-bucket.s3.us-east-1.amazonaws.com</DomainName>
        <OriginAccessControlId>E2ABCDEF123456</OriginAccessControlId>
        <S3OriginConfig><OriginAccessIdentity/></S3OriginConfig>
      </Origin>
    </Items>
  </Origins>
  <DefaultCacheBehavior>
    <TargetOriginId>primary</TargetOriginId>
    <ViewerProtocolPolicy>redirect-to-https</ViewerProtocolPolicy>
    <CachePolicyId>658327ea-f89d-4fab-a63d-7e88639e58f6</CachePolicyId>
  </DefaultCacheBehavior>
  <Comment>Demo distribution</Comment>
  <Enabled>true</Enabled>
  <PriceClass>PriceClass_All</PriceClass>
</DistributionConfig>
```

An `UpdateDistribution` request:

```http
PUT /2020-05-31/distribution/E1ABCDEF123456/config HTTP/1.1
Content-Type: application/xml
If-Match: E2QWASDZXCVB
Authorization: ...

<?xml version="1.0" encoding="UTF-8"?>
<DistributionConfig xmlns="...">
  ... full config, with CallerReference preserved ...
</DistributionConfig>
```

### 6.3 Response Anatomy

```http
HTTP/1.1 201 Created
Content-Type: application/xml
ETag: E2QWASDZXCVB
Location: https://cloudfront.amazonaws.com/2020-05-31/distribution/E1ABCDEF123456

<?xml version="1.0" encoding="UTF-8"?>
<Distribution xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Id>E1ABCDEF123456</Id>
  <ARN>arn:aws:cloudfront::000000000000:distribution/E1ABCDEF123456</ARN>
  <Status>InProgress</Status>
  <LastModifiedTime>2026-04-18T10:30:00.000Z</LastModifiedTime>
  <DomainName>d1abcdef123456.cloudfront.net</DomainName>
  <InProgressInvalidationBatches>0</InProgressInvalidationBatches>
  <ActiveTrustedSigners><Enabled>false</Enabled><Quantity>0</Quantity></ActiveTrustedSigners>
  <ActiveTrustedKeyGroups><Enabled>false</Enabled><Quantity>0</Quantity></ActiveTrustedKeyGroups>
  <DistributionConfig>
    ... echo of submitted config ...
  </DistributionConfig>
</Distribution>
```

Note: `ETag` is a top-level HTTP response header; it is also used as the `If-Match` value on subsequent mutations.

### 6.4 Error Response Format

```http
HTTP/1.1 412 Precondition Failed
Content-Type: text/xml
x-amzn-ErrorType: PreconditionFailed

<?xml version="1.0"?>
<ErrorResponse xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Error>
    <Type>Sender</Type>
    <Code>PreconditionFailed</Code>
    <Message>The If-Match version is missing or not valid for the resource.</Message>
  </Error>
  <RequestId>00000000-0000-0000-0000-000000000000</RequestId>
</ErrorResponse>
```

This format differs from S3's `<Error>` root; CloudFront wraps in `<ErrorResponse>`. The codegen and HTTP layer handle both forms via a protocol-level flag.

### 6.5 Operation-to-Route Mapping (Abridged — Phase 0 + Phase 1)

| Operation | Method | Path | Success | ETag? | If-Match? | Phase |
|-----------|--------|------|---------|-------|-----------|-------|
| **CreateDistribution** | POST | `/2020-05-31/distribution` | 201 | Yes | No | 0 |
| **CreateDistributionWithTags** | POST | `/2020-05-31/distribution?WithTags` | 201 | Yes | No | 0 |
| **GetDistribution** | GET | `/2020-05-31/distribution/{Id}` | 200 | Yes | No | 0 |
| **GetDistributionConfig** | GET | `/2020-05-31/distribution/{Id}/config` | 200 | Yes | No | 0 |
| **UpdateDistribution** | PUT | `/2020-05-31/distribution/{Id}/config` | 200 | Yes | **Yes** | 0 |
| **DeleteDistribution** | DELETE | `/2020-05-31/distribution/{Id}` | 204 | No | **Yes** | 0 |
| **ListDistributions** | GET | `/2020-05-31/distribution` | 200 | No | No | 0 |
| **CopyDistribution** | POST | `/2020-05-31/distribution/{PrimaryDistributionId}/copy` | 201 | Yes | No | 0 |
| **CreateInvalidation** | POST | `/2020-05-31/distribution/{DistributionId}/invalidation` | 201 | No | No | 0 |
| **GetInvalidation** | GET | `/2020-05-31/distribution/{DistributionId}/invalidation/{Id}` | 200 | No | No | 0 |
| **ListInvalidations** | GET | `/2020-05-31/distribution/{DistributionId}/invalidation` | 200 | No | No | 0 |
| **CreateOriginAccessControl** | POST | `/2020-05-31/origin-access-control` | 201 | Yes | No | 0 |
| **GetOriginAccessControl** | GET | `/2020-05-31/origin-access-control/{Id}` | 200 | Yes | No | 0 |
| **GetOriginAccessControlConfig** | GET | `/2020-05-31/origin-access-control/{Id}/config` | 200 | Yes | No | 0 |
| **UpdateOriginAccessControl** | PUT | `/2020-05-31/origin-access-control/{Id}/config` | 200 | Yes | **Yes** | 0 |
| **DeleteOriginAccessControl** | DELETE | `/2020-05-31/origin-access-control/{Id}` | 204 | No | **Yes** | 0 |
| **ListOriginAccessControls** | GET | `/2020-05-31/origin-access-control` | 200 | No | No | 0 |
| **TagResource** | POST | `/2020-05-31/tagging?Operation=Tag` | 204 | No | No | 0 |
| **UntagResource** | POST | `/2020-05-31/tagging?Operation=Untag` | 204 | No | No | 0 |
| **ListTagsForResource** | GET | `/2020-05-31/tagging` | 200 | No | No | 0 |
| **CreateCloudFrontOriginAccessIdentity** | POST | `/2020-05-31/origin-access-identity/cloudfront` | 201 | Yes | No | 1 |
| **GetCloudFrontOriginAccessIdentity** | GET | `/2020-05-31/origin-access-identity/cloudfront/{Id}` | 200 | Yes | No | 1 |
| **UpdateCloudFrontOriginAccessIdentity** | PUT | `/2020-05-31/origin-access-identity/cloudfront/{Id}/config` | 200 | Yes | **Yes** | 1 |
| **DeleteCloudFrontOriginAccessIdentity** | DELETE | `/2020-05-31/origin-access-identity/cloudfront/{Id}` | 204 | No | **Yes** | 1 |
| **ListCloudFrontOriginAccessIdentities** | GET | `/2020-05-31/origin-access-identity/cloudfront` | 200 | No | No | 1 |
| **CreateCachePolicy** | POST | `/2020-05-31/cache-policy` | 201 | Yes | No | 1 |
| ... (all other Policy/KeyGroup/PublicKey/Function operations) | ... | ... | ... | ... | ... | 1-2 |

The full ~90-operation route table is emitted by the codegen from `cloudfront.json`. The table above illustrates the pattern; see §7 for generation details.

### 6.6 Query-Parameter Dispatch Edge Cases

Two patterns require special handling in the router:

- **CreateDistributionWithTags** shares `POST /2020-05-31/distribution` with **CreateDistribution**. Disambiguated by the presence of the `?WithTags` query parameter.
- **TagResource / UntagResource** both target `POST /2020-05-31/tagging`. Disambiguated by `?Operation=Tag` vs `?Operation=Untag`.
- **ListInvalidations** and **CreateInvalidation** share `/2020-05-31/distribution/{Id}/invalidation`; disambiguated by GET vs POST.
- **UpdateDistributionWithStagingConfig** uses `POST /2020-05-31/distribution/{Id}/promote-staging-config`.

The generated route table represents these as distinct entries ordered from most-specific (with query flags) to least-specific so the router picks the right one.

---

## 7. Smithy Code Generation Strategy

### 7.1 Approach: Extend restXml Codegen (First Non-S3 restXml Service)

Today, `codegen/src/codegen.rs` supports restXml only for S3, with some S3-specific assumptions (`S3Request<T>`, `StreamingBlob`, bucket/key binding). Adding CloudFront requires generalizing restXml generation into two modes:

1. **S3-style restXml** (existing): opaque request/response bodies, streaming blob support, bucket sub-resource conventions.
2. **Standard restXml** (new): XML-only request/response bodies, config-wrapper unwrapping, ETag/IfMatch HTTP binding, global service ARN formatting.

This is done by extending `codegen/services/s3.toml` with an `[restxml]` sub-section that CloudFront's TOML can set differently:

```toml
# codegen/services/cloudfront.toml

[service]
name = "cloudfront"
display_name = "CloudFront"
rust_prefix = "CloudFront"
namespace = "com.amazonaws.cloudfront"
protocol = "restXml"

[protocol]
serde_rename = "none"           # XML is not serde-driven; hand-written serializers
emit_http_bindings = true
emit_serde_derives = false
emit_request_wrapper = false    # CloudFront does not need S3Request<T>/StreamingBlob
emit_etag_binding = true        # Operations with config get generate ETag-bearing responses
emit_if_match_binding = true    # Update/Delete require IfMatch header binding
xml_namespace = "http://cloudfront.amazonaws.com/doc/2020-05-31/"
error_xml_wrapper = "ErrorResponse"  # CloudFront wraps errors in <ErrorResponse>; S3 does not

[global]
is_global_service = true
arn_includes_region = false     # arn:aws:cloudfront::{account}:distribution/{id}
sigv4_service_name = "cloudfront"
fixed_region = "us-east-1"

[operations]
phase0 = [
    "CreateDistribution", "CreateDistributionWithTags", "GetDistribution",
    "GetDistributionConfig", "UpdateDistribution", "DeleteDistribution",
    "ListDistributions", "CopyDistribution",
    "CreateInvalidation", "GetInvalidation", "ListInvalidations",
    "CreateOriginAccessControl", "GetOriginAccessControl",
    "GetOriginAccessControlConfig", "UpdateOriginAccessControl",
    "DeleteOriginAccessControl", "ListOriginAccessControls",
    "TagResource", "UntagResource", "ListTagsForResource",
]
phase1 = [
    "CreateCloudFrontOriginAccessIdentity", "GetCloudFrontOriginAccessIdentity",
    "GetCloudFrontOriginAccessIdentityConfig",
    "UpdateCloudFrontOriginAccessIdentity", "DeleteCloudFrontOriginAccessIdentity",
    "ListCloudFrontOriginAccessIdentities",
    "CreateCachePolicy", "GetCachePolicy", "GetCachePolicyConfig",
    "UpdateCachePolicy", "DeleteCachePolicy", "ListCachePolicies",
    "CreateOriginRequestPolicy", "GetOriginRequestPolicy",
    "GetOriginRequestPolicyConfig", "UpdateOriginRequestPolicy",
    "DeleteOriginRequestPolicy", "ListOriginRequestPolicies",
    "CreateResponseHeadersPolicy", "GetResponseHeadersPolicy",
    "GetResponseHeadersPolicyConfig", "UpdateResponseHeadersPolicy",
    "DeleteResponseHeadersPolicy", "ListResponseHeadersPolicies",
]
phase2 = [
    "CreateKeyGroup", "GetKeyGroup", "GetKeyGroupConfig", "UpdateKeyGroup",
    "DeleteKeyGroup", "ListKeyGroups",
    "CreatePublicKey", "GetPublicKey", "GetPublicKeyConfig",
    "UpdatePublicKey", "DeletePublicKey", "ListPublicKeys",
]
phase3 = [
    "CreateRealtimeLogConfig", "GetRealtimeLogConfig", "UpdateRealtimeLogConfig",
    "DeleteRealtimeLogConfig", "ListRealtimeLogConfigs",
    "CreateFunction", "DescribeFunction", "GetFunction", "UpdateFunction",
    "DeleteFunction", "PublishFunction", "TestFunction", "ListFunctions",
    "CreateFieldLevelEncryptionConfig", "CreateFieldLevelEncryptionProfile",
    "GetFieldLevelEncryption", "GetFieldLevelEncryptionConfig",
    "GetFieldLevelEncryptionProfile", "GetFieldLevelEncryptionProfileConfig",
    "UpdateFieldLevelEncryptionConfig", "UpdateFieldLevelEncryptionProfile",
    "DeleteFieldLevelEncryptionConfig", "DeleteFieldLevelEncryptionProfile",
    "ListFieldLevelEncryptionConfigs", "ListFieldLevelEncryptionProfiles",
    "CreateMonitoringSubscription", "GetMonitoringSubscription",
    "DeleteMonitoringSubscription",
    "CreateKeyValueStore", "DescribeKeyValueStore", "UpdateKeyValueStore",
    "DeleteKeyValueStore", "ListKeyValueStores",
]
phase4 = [
    "CreateContinuousDeploymentPolicy", "GetContinuousDeploymentPolicy",
    "GetContinuousDeploymentPolicyConfig", "UpdateContinuousDeploymentPolicy",
    "DeleteContinuousDeploymentPolicy", "ListContinuousDeploymentPolicies",
    "UpdateDistributionWithStagingConfig",
    "AssociateAlias", "ListConflictingAliases",
    "CreateStreamingDistribution", "CreateStreamingDistributionWithTags",
    "GetStreamingDistribution", "GetStreamingDistributionConfig",
    "UpdateStreamingDistribution", "DeleteStreamingDistribution",
    "ListStreamingDistributions",
    "CreateAnycastIpList", "GetAnycastIpList", "UpdateAnycastIpList",
    "DeleteAnycastIpList", "ListAnycastIpLists",
    "CreateVpcOrigin", "GetVpcOrigin", "UpdateVpcOrigin",
    "DeleteVpcOrigin", "ListVpcOrigins",
    "GetResourcePolicy", "PutResourcePolicy", "DeleteResourcePolicy",
    "GetManagedCertificateDetails", "VerifyDnsConfiguration",
    "ListDistributionsByCachePolicyId", "ListDistributionsByKeyGroup",
    "ListDistributionsByOriginRequestPolicyId",
    "ListDistributionsByRealtimeLogConfig",
    "ListDistributionsByResponseHeadersPolicyId",
    "ListDistributionsByVpcOriginId", "ListDistributionsByWebACLId",
    "ListDistributionsByAnycastIpListId",
    "AssociateDistributionWebACL", "DisassociateDistributionWebACL",
    # Tenant / ConnectionFunction / ConnectionGroup / TrustStore — stubs only in Phase 4
    "CreateDistributionTenant", "GetDistributionTenant",
    "GetDistributionTenantByDomain", "UpdateDistributionTenant",
    "DeleteDistributionTenant", "ListDistributionTenants",
    "ListDistributionTenantsByCustomization",
    "CreateInvalidationForDistributionTenant",
    "GetInvalidationForDistributionTenant",
    "ListInvalidationsForDistributionTenant",
    "AssociateDistributionTenantWebACL",
    "DisassociateDistributionTenantWebACL",
    "CreateTrustStore", "GetTrustStore", "UpdateTrustStore",
    "DeleteTrustStore", "ListTrustStores",
    "ListDistributionsByTrustStore", "ListDomainConflicts",
    "UpdateDomainAssociation",
    "CreateConnectionGroup", "GetConnectionGroup",
    "GetConnectionGroupByRoutingEndpoint", "UpdateConnectionGroup",
    "DeleteConnectionGroup", "ListConnectionGroups",
    "CreateConnectionFunction", "DescribeConnectionFunction",
    "GetConnectionFunction", "UpdateConnectionFunction",
    "DeleteConnectionFunction", "PublishConnectionFunction",
    "TestConnectionFunction", "ListConnectionFunctions",
    "ListDistributionsByConnectionFunction",
    "ListDistributionsByConnectionMode",
    "ListDistributionsByOwnedResource",
]

[operations.categories]
distribution = ["CreateDistribution", "CreateDistributionWithTags", "GetDistribution",
    "GetDistributionConfig", "UpdateDistribution", "DeleteDistribution",
    "ListDistributions", "CopyDistribution",
    "UpdateDistributionWithStagingConfig", "AssociateAlias"]
invalidation = ["CreateInvalidation", "GetInvalidation", "ListInvalidations"]
origin_access = ["CreateOriginAccessControl", "GetOriginAccessControl",
    "GetOriginAccessControlConfig", "UpdateOriginAccessControl",
    "DeleteOriginAccessControl", "ListOriginAccessControls",
    "CreateCloudFrontOriginAccessIdentity", "GetCloudFrontOriginAccessIdentity",
    "GetCloudFrontOriginAccessIdentityConfig",
    "UpdateCloudFrontOriginAccessIdentity",
    "DeleteCloudFrontOriginAccessIdentity",
    "ListCloudFrontOriginAccessIdentities"]
policy = [...]    # cache/origin/response headers
key_material = [...]  # key groups, public keys
functions = [...]
realtime_log = [...]
fle = [...]
monitoring = [...]
kvstore = [...]
continuous_deployment = [...]
streaming_distribution = [...]
anycast_ip = [...]
vpc_origin = [...]
resource_policy = [...]
tagging = ["TagResource", "UntagResource", "ListTagsForResource"]
tenant = [...]
trust_store = [...]
connection = [...]

[output]
file_layout = "categorized"
dir = "../crates/rustack-cloudfront-model/src"
```

### 7.2 Generated Route Metadata

```rust
pub struct CloudFrontRoute {
    pub method: http::Method,
    pub path_pattern: &'static str,
    pub query_flag: Option<&'static str>,   // e.g. Some("WithTags") or Some("Operation=Tag")
    pub operation: CloudFrontOperation,
    pub success_status: u16,
    pub emits_etag: bool,
    pub requires_if_match: bool,
}

pub const CLOUDFRONT_ROUTES: &[CloudFrontRoute] = &[
    CloudFrontRoute {
        method: http::Method::POST,
        path_pattern: "/2020-05-31/distribution",
        query_flag: Some("WithTags"),
        operation: CloudFrontOperation::CreateDistributionWithTags,
        success_status: 201,
        emits_etag: true,
        requires_if_match: false,
    },
    CloudFrontRoute {
        method: http::Method::POST,
        path_pattern: "/2020-05-31/distribution",
        query_flag: None,
        operation: CloudFrontOperation::CreateDistribution,
        success_status: 201,
        emits_etag: true,
        requires_if_match: false,
    },
    // ... ~90 more entries
];
```

### 7.3 XML Codegen: Structure Handling

Key generator behaviors specific to CloudFront (beyond what S3 needed):

1. **Config-wrapper unwrap** -- `DistributionConfig`, `CachePolicyConfig`, etc. are the request body root element; generated code must unwrap them from the body into `...Input` structs.
2. **Quantity/Items lists** -- CloudFront XML lists are wrapped as `<Aliases><Quantity>N</Quantity><Items><CNAME>...</CNAME></Items></Aliases>`. The codegen emits a helper that reads `Quantity` and `Items` into `Vec<T>`, and writes them back in the same shape. This is a CloudFront-specific but well-documented pattern; the helper lives in `rustack-restxml::cloudfront_list`.
3. **Enabled/Quantity wrappers** -- e.g., `<ActiveTrustedKeyGroups><Enabled>false</Enabled><Quantity>0</Quantity><Items>...</Items></ActiveTrustedKeyGroups>`.
4. **ETag HTTP-bound output** -- `@httpResponseCode` and `@httpHeader("ETag")` Smithy traits map to response metadata on the generated output struct.
5. **IfMatch HTTP-bound input** -- `@httpHeader("If-Match")` maps to an `if_match: Option<String>` field on the input.
6. **Error XML wrapper** -- error serializer wraps in `<ErrorResponse>...</ErrorResponse>` whereas S3's wraps in `<Error>` directly.

### 7.4 Generated Output Estimate

- ~90 input structs
- ~90 output structs
- ~250 nested config types (`DistributionConfig`, `CacheBehavior`, `Origin`, `ForwardedValues`, `Cookies`, `Headers`, `QueryStringCacheKeys`, `ViewerCertificate`, `Restrictions`, `GeoRestriction`, `CustomErrorResponses`, `CustomErrorResponse`, `Origins`, `OriginCustomHeaders`, `OriginShield`, `OriginGroup`, `OriginGroupMember`, `CacheBehaviors`, `AllowedMethods`, `CachedMethods`, `LambdaFunctionAssociations`, `FunctionAssociations`, `TrustedSigners`, `TrustedKeyGroups`, ...)
- 1 operation enum with ~90 variants
- 1 route table with ~90 entries
- ~40 error types (`AccessDenied`, `CNAMEAlreadyExists`, `DistributionNotDisabled`, `IllegalUpdate`, `InconsistentQuantities`, `InvalidArgument`, `InvalidIfMatchVersion`, `NoSuchDistribution`, `NoSuchInvalidation`, `NoSuchOriginAccessControl`, `PreconditionFailed`, `TooManyDistributionCNAMEs`, ...)

Total: roughly 10,000–13,000 lines of generated code.

### 7.5 Smithy Model Acquisition

```makefile
codegen-download: codegen-download-cloudfront

codegen-download-cloudfront:
	@curl -sL https://raw.githubusercontent.com/aws/aws-sdk-js-v3/main/codegen/sdk-codegen/aws-models/cloudfront.json \
		-o codegen/smithy-model/cloudfront.json

codegen-cloudfront:
	@cd codegen && cargo run -- --service cloudfront
	@cargo +nightly fmt -p rustack-cloudfront-model

codegen: codegen-s3 codegen-dynamodb ... codegen-cloudfront
```

The file `codegen/smithy-model/cloudfront.json` is checked in (like the other service models).

---

## 8. Crate Structure

### 8.1 New Crate: `rustack-restxml` (extracted)

Before CloudFront: all XML code lives in `rustack-s3-xml`. As part of this work, extract the protocol-agnostic pieces (writer helpers, scanner, escaping, namespace handling, typed serialize/deserialize traits, quantity-items list helpers) into `rustack-restxml`, and keep only S3-specific types (`AccessControlPolicy`, `CompleteMultipartUploadRequest`, etc.) in `rustack-s3-xml`.

```
crates/rustack-restxml/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── writer.rs            # typed XML writer, namespace support
    ├── reader.rs            # typed XML reader, event-based
    ├── escape.rs            # XML entity escaping
    ├── datetime.rs          # ISO-8601 and RFC-1123 helpers
    ├── quantity_items.rs    # CloudFront-style <Quantity><Items>...</Items></> list
    ├── error.rs             # ErrorResponse / Error wrapper helpers (both forms)
    └── traits.rs            # ToXml, FromXml traits + derive-helper macros
```

**Dependencies**: `quick-xml`, `bytes`, `chrono`, `base64`.

`rustack-s3-xml` now depends on `rustack-restxml`. This is a refactor-in-place; S3 tests must continue to pass byte-for-byte.

### 8.2 New Crate: `rustack-cloudfront-model` (auto-generated)

```
crates/rustack-cloudfront-model/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── types/
    │   ├── mod.rs
    │   ├── distribution_config.rs   # DistributionConfig + all nested config types
    │   ├── cache_behavior.rs        # CacheBehavior, DefaultCacheBehavior, ForwardedValues
    │   ├── origin.rs                # Origin, Origins, OriginCustomHeaders, S3OriginConfig, CustomOriginConfig
    │   ├── origin_access.rs         # OriginAccessControlConfig, OriginAccessIdentityConfig, CloudFrontOriginAccessIdentity
    │   ├── policy.rs                # CachePolicyConfig, OriginRequestPolicyConfig, ResponseHeadersPolicyConfig
    │   ├── invalidation.rs          # InvalidationBatch, Paths
    │   ├── aliases.rs               # Aliases, AlternateDomainName
    │   ├── viewer_certificate.rs
    │   ├── restrictions.rs          # Restrictions, GeoRestriction
    │   ├── logging.rs               # LoggingConfig, RealtimeLogConfig, EndPoint
    │   ├── key_material.rs          # KeyGroupConfig, PublicKeyConfig, KeyValueStore
    │   ├── function.rs              # FunctionConfig, FunctionSummary, FunctionMetadata
    │   ├── fle.rs                   # FieldLevelEncryptionConfig, FieldLevelEncryptionProfileConfig
    │   ├── monitoring.rs            # MonitoringSubscription
    │   ├── tag.rs                   # Tag, Tags
    │   └── common.rs                # Status, PriceClass, HttpVersion, etc.
    ├── input/
    │   ├── mod.rs
    │   ├── distribution.rs
    │   ├── invalidation.rs
    │   ├── origin_access.rs
    │   ├── policy.rs
    │   ├── key_material.rs
    │   ├── function.rs
    │   ├── fle.rs
    │   ├── realtime_log.rs
    │   ├── monitoring.rs
    │   ├── kvstore.rs
    │   ├── continuous_deployment.rs
    │   ├── streaming_distribution.rs
    │   ├── anycast_ip.rs
    │   ├── vpc_origin.rs
    │   ├── resource_policy.rs
    │   ├── tagging.rs
    │   ├── tenant.rs
    │   ├── trust_store.rs
    │   └── connection.rs
    ├── output/
    │   └── ... (mirrors input/)
    ├── operations.rs                # CloudFrontOperation enum + route table
    └── error.rs                     # CloudFrontError + CloudFrontErrorCode
```

**Dependencies**: `serde` (for derive), `chrono`, `http`, `rustack-restxml`.

### 8.3 New Crate: `rustack-cloudfront-http`

```
crates/rustack-cloudfront-http/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── router.rs        # URL + method + query-flag pattern matching
    ├── dispatch.rs      # CloudFrontHandler trait + dispatch table
    ├── service.rs       # Hyper Service impl
    ├── request.rs       # restXml request deserialization (body + path + headers)
    ├── response.rs      # restXml response serialization (body + ETag + Location)
    ├── error.rs         # Error response formatting (<ErrorResponse>...)
    └── body.rs          # Response body type
```

**Dependencies**: `rustack-cloudfront-model`, `rustack-restxml`, `rustack-auth`, `hyper`, `http`, `bytes`.

### 8.4 New Crate: `rustack-cloudfront-core`

```
crates/rustack-cloudfront-core/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── config.rs             # CloudFrontConfig
    ├── provider.rs           # RustackCloudFront: owns the store, spawns lifecycle tasks
    ├── handler.rs            # CloudFrontHandler impl that bridges HTTP to provider
    ├── error.rs              # CloudFrontServiceError + conversion into model errors
    ├── storage.rs            # CloudFrontStore: DashMaps for each resource kind
    ├── ids.rs                # ID generation (14-char uppercase alphanumeric, prefixed)
    ├── etag.rs               # ETag generation and validation
    ├── arn.rs                # ARN construction (no region)
    ├── lifecycle/
    │   ├── mod.rs
    │   ├── distribution.rs   # InProgress → Deployed state machine
    │   ├── invalidation.rs   # InProgress → Completed state machine
    │   └── function.rs       # DEVELOPMENT → LIVE for CloudFront Functions
    ├── s3_integration.rs     # Optional: OAC → S3 bucket policy write-through
    └── ops/
        ├── mod.rs
        ├── distribution.rs
        ├── invalidation.rs
        ├── origin_access_control.rs
        ├── origin_access_identity.rs
        ├── cache_policy.rs
        ├── origin_request_policy.rs
        ├── response_headers_policy.rs
        ├── key_group.rs
        ├── public_key.rs
        ├── realtime_log_config.rs
        ├── function.rs
        ├── fle.rs
        ├── monitoring_subscription.rs
        ├── kvstore.rs
        ├── continuous_deployment.rs
        ├── streaming_distribution.rs
        ├── anycast_ip.rs
        ├── vpc_origin.rs
        ├── resource_policy.rs
        ├── tagging.rs
        ├── tenant.rs
        ├── trust_store.rs
        └── connection.rs
```

**Dependencies**: `rustack-core`, `rustack-cloudfront-model`, `rustack-s3-core` (optional, feature `s3-integration`), `dashmap`, `tokio`, `uuid`, `chrono`, `rand`, `tracing`.

### 8.5 Workspace Changes

```toml
[workspace.dependencies]
# ... existing ...
rustack-restxml = { path = "crates/rustack-restxml" }
rustack-cloudfront-model = { path = "crates/rustack-cloudfront-model" }
rustack-cloudfront-http = { path = "crates/rustack-cloudfront-http" }
rustack-cloudfront-core = { path = "crates/rustack-cloudfront-core" }
```

The `rustack-cli` (formerly `rustack`) app gains a `cloudfront` cargo feature flag, on by default alongside the other services.

---

## 9. HTTP Layer Design

### 9.1 Router

```rust
pub struct CloudFrontRouter;

impl CloudFrontRouter {
    /// Resolve an HTTP request to a CloudFront operation.
    ///
    /// Returns the matched operation and extracted path parameters.
    pub fn resolve(
        method: &http::Method,
        path: &str,
        query: &str,
    ) -> Result<(CloudFrontOperation, PathParams), CloudFrontError> {
        for route in CLOUDFRONT_ROUTES {
            if *method != route.method {
                continue;
            }
            if let Some(flag) = route.query_flag {
                if !query_contains_flag(query, flag) {
                    continue;
                }
            }
            if let Some(params) = match_path(path, route.path_pattern) {
                return Ok((route.operation, params));
            }
        }
        Err(CloudFrontError::unknown_operation(method, path))
    }
}

fn query_contains_flag(query: &str, flag: &str) -> bool {
    // `flag` may be "WithTags" (key with no value) or "Operation=Tag" (key=value).
    if flag.contains('=') {
        query.split('&').any(|kv| kv == flag)
    } else {
        query.split('&').any(|kv| kv == flag || kv.starts_with(&format!("{flag}=")))
    }
}
```

Routes are ordered **most-specific-first** (those with `query_flag` come before their flag-less sibling).

### 9.2 Handler Trait

```rust
pub trait CloudFrontHandler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: CloudFrontOperation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<CloudFrontResponseBody>, CloudFrontError>> + Send>>;
}
```

### 9.3 Request Deserialization

1. Parse path params from `path_pattern` vs actual path.
2. Extract HTTP-bound inputs: `If-Match` header, `Id` path segment, etc.
3. If the operation has a body, parse the XML into the config-wrapper type (`DistributionConfig`, etc.) via `rustack-restxml`.
4. Map parsed body + path + header inputs into the operation's `...Input` struct.

### 9.4 Response Serialization

1. Serialize output struct to XML via `rustack-restxml`.
2. Set `Content-Type: application/xml; charset=utf-8`.
3. If the operation emits ETag, set `ETag: {etag}` header from the output's `etag` field.
4. Set success status code from the route (201, 200, 204 per operation).
5. For `Create*`, set `Location: https://cloudfront.amazonaws.com/2020-05-31/{resource}/{id}` header per AWS convention.

---

## 10. Storage Engine Design

### 10.1 Overview

One `DashMap` per top-level resource kind. Each mapped value is a `VersionedRecord<T>` carrying the resource plus its current ETag and any status-machine state.

### 10.2 Versioned Record

```rust
#[derive(Debug, Clone)]
pub struct VersionedRecord<T> {
    pub resource: T,
    pub etag: String,
    pub created_time: chrono::DateTime<chrono::Utc>,
    pub last_modified_time: chrono::DateTime<chrono::Utc>,
}

impl<T> VersionedRecord<T> {
    pub fn new(resource: T) -> Self {
        let now = chrono::Utc::now();
        Self {
            resource,
            etag: generate_etag(),
            created_time: now,
            last_modified_time: now,
        }
    }

    /// Bump the ETag on update. Preserves `created_time`.
    pub fn updated(self, resource: T) -> Self {
        Self {
            resource,
            etag: generate_etag(),
            created_time: self.created_time,
            last_modified_time: chrono::Utc::now(),
        }
    }
}

fn generate_etag() -> String {
    // 13-char uppercase alphanumeric, matches AWS format: "E2QWASDZXCVB"
    let mut rng = rand::thread_rng();
    std::iter::once('E')
        .chain((0..12).map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 { (b'0' + idx) as char } else { (b'A' + idx - 10) as char }
        }))
        .collect()
}
```

### 10.3 Store Layout

```rust
#[derive(Debug, Default)]
pub struct CloudFrontStore {
    pub distributions: DashMap<String, VersionedRecord<DistributionRecord>>,
    pub invalidations: DashMap<(String, String), InvalidationRecord>,
    //                        ^ (distribution_id, invalidation_id)
    pub origin_access_controls: DashMap<String, VersionedRecord<OriginAccessControlRecord>>,
    pub origin_access_identities: DashMap<String, VersionedRecord<OriginAccessIdentityRecord>>,
    pub cache_policies: DashMap<String, VersionedRecord<CachePolicyRecord>>,
    pub origin_request_policies: DashMap<String, VersionedRecord<OriginRequestPolicyRecord>>,
    pub response_headers_policies: DashMap<String, VersionedRecord<ResponseHeadersPolicyRecord>>,
    pub key_groups: DashMap<String, VersionedRecord<KeyGroupRecord>>,
    pub public_keys: DashMap<String, VersionedRecord<PublicKeyRecord>>,
    pub realtime_log_configs: DashMap<String, VersionedRecord<RealtimeLogConfigRecord>>,
    pub functions: DashMap<String, FunctionRecord>,  // has its own ETag/state machine
    pub fle_configs: DashMap<String, VersionedRecord<FleConfigRecord>>,
    pub fle_profiles: DashMap<String, VersionedRecord<FleProfileRecord>>,
    pub monitoring_subscriptions: DashMap<String, MonitoringSubscription>,
    //                                    ^ keyed by distribution_id
    pub kv_stores: DashMap<String, VersionedRecord<KeyValueStoreRecord>>,
    pub continuous_deployment_policies: DashMap<String, VersionedRecord<ContinuousDeploymentPolicyRecord>>,
    pub streaming_distributions: DashMap<String, VersionedRecord<StreamingDistributionRecord>>,
    pub tags: DashMap<String, HashMap<String, String>>,  // keyed by resource ARN
    // ... (Phase 4: tenants, trust stores, vpc origins, anycast IPs, connection functions, etc.)
}
```

All management mutations go through methods on `RustackCloudFront`; no storage type is exposed publicly.

### 10.4 Core Record Types

```rust
#[derive(Debug, Clone)]
pub struct DistributionRecord {
    pub id: String,                     // 14-char uppercase, e.g., "E1ABCDEF123456"
    pub arn: String,                    // arn:aws:cloudfront::{account}:distribution/{id}
    pub domain_name: String,            // {lower(id)}.cloudfront.net  (not routable)
    pub caller_reference: String,       // client-provided idempotency token
    pub status: DistributionStatus,     // InProgress | Deployed
    pub config: DistributionConfig,     // echoed back on reads
    pub in_progress_invalidation_batches: u32,
    pub active_trusted_signers: TrustedSigners,
    pub active_trusted_key_groups: TrustedKeyGroups,
    pub alias_icp_recordals: Vec<AliasIcpRecordal>,   // Chinese ICP stuff; empty by default
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionStatus { InProgress, Deployed }

#[derive(Debug, Clone)]
pub struct InvalidationRecord {
    pub id: String,                     // 14-char uppercase, e.g., "I2JCJI9THKVOAJ"
    pub distribution_id: String,
    pub status: InvalidationStatus,     // InProgress | Completed
    pub create_time: chrono::DateTime<chrono::Utc>,
    pub invalidation_batch: InvalidationBatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationStatus { InProgress, Completed }

#[derive(Debug, Clone)]
pub struct OriginAccessControlRecord {
    pub id: String,                     // 14-char, prefixed "E"
    pub config: OriginAccessControlConfig,
}

#[derive(Debug, Clone)]
pub struct FunctionRecord {
    pub name: String,
    pub status: FunctionStatus,         // UNPUBLISHED | UNASSOCIATED | ASSOCIATED
    pub stage: FunctionStage,           // DEVELOPMENT | LIVE
    pub function_code: Bytes,           // opaque blob (base64-decoded)
    pub function_config: FunctionConfig,
    pub etag_development: String,
    pub etag_live: Option<String>,
    pub created_time: chrono::DateTime<chrono::Utc>,
    pub last_modified_time: chrono::DateTime<chrono::Utc>,
}
```

### 10.5 ID Generation

CloudFront IDs are 14-character uppercase alphanumeric, typically prefixed with a letter that identifies the resource kind: distributions begin with `E`, invalidations with `I`, OAIs with `E`, OACs with `E`. Cache policies use UUIDs (dashed form). Functions use arbitrary user-provided names.

```rust
pub fn generate_distribution_id() -> String { generate_prefixed_id('E') }
pub fn generate_invalidation_id() -> String { generate_prefixed_id('I') }
pub fn generate_oai_id() -> String { generate_prefixed_id('E') }
pub fn generate_oac_id() -> String { generate_prefixed_id('E') }
pub fn generate_cache_policy_id() -> String { uuid::Uuid::new_v4().to_string() }

fn generate_prefixed_id(prefix: char) -> String {
    let mut rng = rand::thread_rng();
    std::iter::once(prefix)
        .chain((0..13).map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 { (b'0' + idx) as char } else { (b'A' + idx - 10) as char }
        }))
        .collect()
}
```

When `CLOUDFRONT_DETERMINISTIC_IDS=true`, IDs are derived from `sha256(caller_reference)` truncated to 13 chars and prefixed, for snapshot-test stability.

### 10.6 ARN Construction

CloudFront ARNs exclude region (global service):

```rust
fn distribution_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:distribution/{id}")
}

fn origin_access_control_arn(account_id: &str, id: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:origin-access-control/{id}")
}

fn function_arn(account_id: &str, name: &str) -> String {
    format!("arn:aws:cloudfront::{account_id}:function/{name}")
}
```

### 10.7 Domain Name Construction

```rust
fn distribution_domain(id: &str) -> String {
    format!("{}.cloudfront.net", id.to_lowercase())
}
```

A future enhancement (non-goal today) could be configurable, e.g. `CLOUDFRONT_DOMAIN_SUFFIX=cloudfront.rustack.local`.

---

## 11. Core Business Logic

### 11.1 Provider

```rust
pub struct RustackCloudFront {
    store: CloudFrontStore,
    config: Arc<CloudFrontConfig>,
    #[cfg(feature = "s3-integration")]
    s3: Option<Arc<RustackS3>>,
    /// Shutdown flag for background lifecycle tasks.
    shutdown: Arc<AtomicBool>,
    /// Handles for background tasks (for graceful shutdown).
    task_handles: Mutex<Vec<JoinHandle<()>>>,
}

pub struct CloudFrontConfig {
    pub skip_signature_validation: bool,
    pub default_region: String,        // always "us-east-1"
    pub account_id: String,
    /// Milliseconds before a Distribution transitions from InProgress to Deployed.
    pub distribution_propagation_ms: u64,
    /// Milliseconds before an Invalidation transitions from InProgress to Completed.
    pub invalidation_completion_ms: u64,
    pub deterministic_ids: bool,
    /// Domain suffix for generated distribution domain names.
    pub domain_suffix: String,
}

impl CloudFrontConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("CLOUDFRONT_SKIP_SIGNATURE_VALIDATION", true),
            default_region: "us-east-1".to_string(),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            distribution_propagation_ms: env_u64("CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS", 0),
            invalidation_completion_ms: env_u64("CLOUDFRONT_INVALIDATION_COMPLETION_MS", 0),
            deterministic_ids: env_bool("CLOUDFRONT_DETERMINISTIC_IDS", false),
            domain_suffix: env_str("CLOUDFRONT_DOMAIN_SUFFIX", "cloudfront.net"),
        }
    }
}
```

### 11.2 Phased Operations

#### Phase 0 (20 operations) — Distribution + Invalidation + OAC + Tags

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `CreateDistribution` | High | Generate ID, allocate domain, validate config (origin URIs, cache behavior target matches an origin, viewer certificate), reject duplicate `CallerReference`, persist with `InProgress` status, spawn transition task |
| `CreateDistributionWithTags` | Medium | Same as above + initial tag set |
| `GetDistribution` | Low | Lookup, return with current ETag header |
| `GetDistributionConfig` | Low | Lookup, return only the config (strips status/ARN) with ETag |
| `UpdateDistribution` | High | Validate `If-Match` against stored ETag; replace config (partial updates are NOT supported — AWS requires full config replace); validate origins/behaviors/referenced policies exist; bump ETag; re-run transition task |
| `DeleteDistribution` | Medium | Validate `If-Match`; require `Enabled=false` in stored config (else return `DistributionNotDisabled`); remove; mark related invalidations history preserved |
| `ListDistributions` | Low | Paginated list with optional `Marker` and `MaxItems` |
| `CopyDistribution` | Medium | Deep-clone source distribution's config, generate new ID/ARN/domain, preserve `CallerReference` from input |
| `CreateInvalidation` | Medium | Validate `distribution_id` exists; generate invalidation ID; store with `InProgress`; spawn transition task |
| `GetInvalidation` | Low | Lookup by `(distribution_id, invalidation_id)` |
| `ListInvalidations` | Low | List for a distribution, ordered by create_time desc |
| `CreateOriginAccessControl` | Medium | Generate ID, validate `SigningProtocol`, `SigningBehavior`, `OriginAccessControlOriginType` |
| `GetOriginAccessControl` | Low | Lookup |
| `GetOriginAccessControlConfig` | Low | Return config portion only |
| `UpdateOriginAccessControl` | Medium | Validate `If-Match`, apply config |
| `DeleteOriginAccessControl` | Medium | Validate `If-Match`, reject if referenced by any distribution (`OriginAccessControlInUse`) |
| `ListOriginAccessControls` | Low | Paginated |
| `TagResource` | Low | Validate ARN kind; merge tag keys |
| `UntagResource` | Low | Remove specified keys |
| `ListTagsForResource` | Low | Return tag map |

#### Phase 1 (24 operations) — OAI + Cache/OriginRequest/ResponseHeaders Policies

Same CRUD+List pattern for each policy kind. Notable cross-resource checks:
- `DeleteCachePolicy` rejects with `CachePolicyInUse` if any distribution's default or per-route cache behavior references the policy ID.
- `DeleteOriginRequestPolicy` similar.
- `DeleteResponseHeadersPolicy` similar.
- `DeleteCloudFrontOriginAccessIdentity` rejects with `CloudFrontOriginAccessIdentityInUse` if referenced.

Managed (AWS-owned, read-only) policies are seeded at provider startup with their well-known IDs: `CachingOptimized` (`658327ea-f89d-4fab-a63d-7e88639e58f6`), `CachingDisabled` (`4135ea2d-6df8-44a3-9df3-4b5a84be39ad`), etc. `UpdateCachePolicy` / `DeleteCachePolicy` on these returns `IllegalUpdate`.

#### Phase 2 (12 operations) — Key Groups + Public Keys

CRUD + List for `KeyGroup` and `PublicKey`. `DeletePublicKey` is rejected with `PublicKeyInUse` if the key is referenced by any KeyGroup. `DeleteKeyGroup` is rejected with `ResourceInUse` if referenced by a distribution's `TrustedKeyGroups`.

#### Phase 3 (28 operations) — Functions + Realtime Logs + FLE + Monitoring + KVStore

- **CloudFront Functions**: state machine is `DEVELOPMENT` (on create/update) → `LIVE` (on `PublishFunction`). `TestFunction` always returns a canned success payload with zero `ComputeUtilization` and empty function output. The function blob is stored as an opaque `Bytes` — no JS parse or execution.
- **Realtime Log Configs**: stored only; no log writing.
- **FLE**: keys, field patterns, profiles stored; no request body encryption occurs (there is no request body handling).
- **MonitoringSubscription**: keyed per distribution; single-entity.
- **KeyValueStore**: metadata stored; the key-value data plane (`PutKey`, `GetKey`) is typically invoked from functions, which we do not run.

#### Phase 4 (rest) — Streaming Distributions + Continuous Deployment + Tenants + VPC Origins + Anycast + TrustStore + Resource Policies + List-by-* variants

Stubs for compatibility. Each maintains minimal state so IaC delete cycles complete cleanly.

### 11.3 CreateDistribution Logic (illustrative)

```rust
impl RustackCloudFront {
    pub fn create_distribution(
        &self,
        input: CreateDistributionInput,
    ) -> Result<CreateDistributionOutput, CloudFrontServiceError> {
        // 1. Validate caller_reference uniqueness (AWS idempotency semantics).
        let caller_ref = &input.distribution_config.caller_reference;
        for entry in self.store.distributions.iter() {
            if entry.resource.caller_reference == *caller_ref {
                return Err(CloudFrontServiceError::DistributionAlreadyExists {
                    message: format!(
                        "The caller reference that you are using to create a distribution is associated with another distribution."
                    ),
                });
            }
        }

        // 2. Validate structure.
        self.validate_distribution_config(&input.distribution_config)?;

        // 3. Allocate ID and derive metadata.
        let id = if self.config.deterministic_ids {
            deterministic_distribution_id(caller_ref)
        } else {
            generate_distribution_id()
        };
        let now = chrono::Utc::now();
        let record = DistributionRecord {
            id: id.clone(),
            arn: distribution_arn(&self.config.account_id, &id),
            domain_name: format!("{}.{}", id.to_lowercase(), self.config.domain_suffix),
            caller_reference: caller_ref.clone(),
            status: DistributionStatus::InProgress,
            config: input.distribution_config.clone(),
            in_progress_invalidation_batches: 0,
            active_trusted_signers: TrustedSigners::disabled(),
            active_trusted_key_groups: TrustedKeyGroups::disabled(),
            alias_icp_recordals: Vec::new(),
        };
        let versioned = VersionedRecord::new(record);
        let etag = versioned.etag.clone();
        let output = self.distribution_to_output(&versioned, &etag, now)?;
        self.store.distributions.insert(id.clone(), versioned);

        // 4. Spawn transition task: InProgress -> Deployed.
        self.spawn_distribution_transition(id);

        Ok(output)
    }

    fn validate_distribution_config(
        &self,
        cfg: &DistributionConfig,
    ) -> Result<(), CloudFrontServiceError> {
        // a. At least one origin.
        if cfg.origins.quantity == 0 {
            return Err(CloudFrontServiceError::InvalidArgument {
                message: "Origins.Quantity must be at least 1".into(),
            });
        }
        // b. DefaultCacheBehavior.TargetOriginId matches some origin.
        let target = &cfg.default_cache_behavior.target_origin_id;
        if !cfg.origins.items.iter().any(|o| o.id == *target) {
            return Err(CloudFrontServiceError::NoSuchOrigin {
                message: format!("Origin with ID {target} does not exist."),
            });
        }
        // c. Each per-route CacheBehavior.TargetOriginId matches some origin.
        for cb in cfg.cache_behaviors.items.iter() {
            if !cfg.origins.items.iter().any(|o| o.id == cb.target_origin_id) {
                return Err(CloudFrontServiceError::NoSuchOrigin {
                    message: format!("Origin with ID {} does not exist.", cb.target_origin_id),
                });
            }
        }
        // d. Referenced CachePolicyId / OriginRequestPolicyId / ResponseHeadersPolicyId exist.
        if let Some(pid) = &cfg.default_cache_behavior.cache_policy_id {
            if !self.store.cache_policies.contains_key(pid) && !is_managed_policy(pid) {
                return Err(CloudFrontServiceError::NoSuchCachePolicy {
                    message: format!("CachePolicy {pid} does not exist."),
                });
            }
        }
        // e. If OAC referenced, it must exist.
        for origin in cfg.origins.items.iter() {
            if let Some(oac_id) = &origin.origin_access_control_id {
                if !oac_id.is_empty() && !self.store.origin_access_controls.contains_key(oac_id) {
                    return Err(CloudFrontServiceError::NoSuchOriginAccessControl {
                        message: format!("OriginAccessControl {oac_id} does not exist."),
                    });
                }
            }
        }
        // f. ViewerCertificate sanity: must have one of CloudFrontDefault or ACM/IAM cert.
        // ... (omitted; follows AWS error mapping)
        Ok(())
    }
}
```

### 11.4 CreateInvalidation Logic

```rust
pub fn create_invalidation(
    &self,
    distribution_id: &str,
    input: CreateInvalidationInput,
) -> Result<CreateInvalidationOutput, CloudFrontServiceError> {
    // Validate distribution exists (regardless of status).
    if !self.store.distributions.contains_key(distribution_id) {
        return Err(CloudFrontServiceError::NoSuchDistribution {
            message: format!("The specified distribution does not exist: {distribution_id}"),
        });
    }

    let batch = &input.invalidation_batch;
    if batch.paths.quantity == 0 {
        return Err(CloudFrontServiceError::InvalidArgument {
            message: "Paths.Quantity must be at least 1".into(),
        });
    }
    // Idempotency check: same CallerReference on same distribution returns existing.
    for entry in self.store.invalidations.iter() {
        if entry.key().0 == distribution_id
            && entry.value().invalidation_batch.caller_reference == batch.caller_reference
        {
            return Ok(self.invalidation_to_output(entry.value()));
        }
    }

    let inv_id = if self.config.deterministic_ids {
        deterministic_invalidation_id(&batch.caller_reference)
    } else {
        generate_invalidation_id()
    };
    let record = InvalidationRecord {
        id: inv_id.clone(),
        distribution_id: distribution_id.to_string(),
        status: InvalidationStatus::InProgress,
        create_time: chrono::Utc::now(),
        invalidation_batch: batch.clone(),
    };
    let output = self.invalidation_to_output(&record);

    // Bump in_progress_invalidation_batches on the distribution.
    if let Some(mut dist) = self.store.distributions.get_mut(distribution_id) {
        dist.resource.in_progress_invalidation_batches += 1;
    }
    self.store.invalidations.insert(
        (distribution_id.to_string(), inv_id.clone()),
        record,
    );

    // Spawn transition task: InProgress -> Completed.
    self.spawn_invalidation_transition(distribution_id.to_string(), inv_id);

    Ok(output)
}
```

### 11.5 Managed Policy Seeding

On provider construction, insert well-known AWS-managed cache policies, origin-request policies, and response-headers policies with their canonical IDs so that configs referencing them validate. Managed policies are flagged read-only; `Update*` or `Delete*` on them returns `IllegalUpdate`.

---

## 12. Error Handling

### 12.1 Error Taxonomy

The generated `CloudFrontErrorCode` enum includes (abridged):

```rust
#[non_exhaustive]
pub enum CloudFrontErrorCode {
    // Authentication / authorization
    AccessDenied,
    MissingAction,
    InvalidAction,
    InvalidSignature,

    // Preconditions
    PreconditionFailed,
    InvalidIfMatchVersion,

    // Not-found family
    NoSuchDistribution,
    NoSuchInvalidation,
    NoSuchOriginAccessControl,
    NoSuchCloudFrontOriginAccessIdentity,
    NoSuchCachePolicy,
    NoSuchOriginRequestPolicy,
    NoSuchResponseHeadersPolicy,
    NoSuchKeyGroup,
    NoSuchPublicKey,
    NoSuchRealtimeLogConfig,
    NoSuchFunctionExists,
    NoSuchResource,
    NoSuchOrigin,
    NoSuchContinuousDeploymentPolicy,
    NoSuchMonitoringSubscription,
    NoSuchStreamingDistribution,
    NoSuchFieldLevelEncryptionConfig,
    NoSuchFieldLevelEncryptionProfile,

    // Conflict / state errors
    DistributionAlreadyExists,
    DistributionNotDisabled,
    StreamingDistributionNotDisabled,
    CNAMEAlreadyExists,
    TooManyDistributionCNAMEs,
    InvalidOrigin,
    InvalidArgument,
    InconsistentQuantities,
    IllegalUpdate,
    InvalidViewerCertificate,

    // In-use errors (deletion blocked)
    CachePolicyInUse,
    OriginRequestPolicyInUse,
    ResponseHeadersPolicyInUse,
    CloudFrontOriginAccessIdentityInUse,
    OriginAccessControlInUse,
    KeyGroupAlreadyExists,
    PublicKeyInUse,
    ResourceInUse,
    FunctionInUse,

    // Quota / too-many
    TooManyDistributions,
    TooManyInvalidationsInProgress,
    TooManyOriginAccessControls,
    TooManyCachePolicies,
    TooManyOriginRequestPolicies,
    TooManyResponseHeadersPolicies,
    TooManyKeyGroups,
    TooManyPublicKeys,
    TooManyRealtimeLogConfigs,
    TooManyFunctions,

    // Catch-all
    InternalFailure,
}
```

The Smithy model's `smithy.api#httpError` traits provide status codes; the codegen populates `status_code` on `CloudFrontError`.

### 12.2 Error Wire Format

```xml
<?xml version="1.0"?>
<ErrorResponse xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/">
  <Error>
    <Type>Sender</Type>
    <Code>NoSuchDistribution</Code>
    <Message>The specified distribution does not exist: E9999...</Message>
  </Error>
  <RequestId>{uuid}</RequestId>
</ErrorResponse>
```

Generated serializer uses `rustack-restxml::error::ErrorResponseWriter`.

### 12.3 `PreconditionFailed` vs `InvalidIfMatchVersion`

- **`InvalidIfMatchVersion`**: `If-Match` header is missing or syntactically invalid.
- **`PreconditionFailed`**: `If-Match` is present and well-formed but does not match the current ETag.

Tests must verify each case independently; Terraform's state reconciliation logic relies on this distinction.

---

## 13. ETag / IfMatch Concurrency Model

### 13.1 Generation

ETags are opaque server-side tokens that change on every mutation. AWS uses 13-char uppercase alphanumeric strings (e.g., `E2QWASDZXCVB1`). Rustack matches this format.

### 13.2 Emission

Every `Get*` and `Get*Config` operation for a versioned resource returns the current ETag as a response header. Every `Create*` returns a new ETag.

### 13.3 Validation

Every `Update*` and every `Delete*` on a versioned resource must include `If-Match`:

```rust
fn require_if_match(
    stored_etag: &str,
    if_match: Option<&str>,
) -> Result<(), CloudFrontServiceError> {
    let value = if_match.map(str::trim).filter(|s| !s.is_empty());
    let Some(v) = value else {
        return Err(CloudFrontServiceError::InvalidIfMatchVersion {
            message: "The If-Match version is missing or not valid.".into(),
        });
    };
    if v != stored_etag {
        return Err(CloudFrontServiceError::PreconditionFailed {
            message: format!("The If-Match version is missing or not valid for the resource."),
        });
    }
    Ok(())
}
```

Applies uniformly across Distribution, OAC, OAI, CachePolicy, OriginRequestPolicy, ResponseHeadersPolicy, KeyGroup, PublicKey, FLE configs/profiles, StreamingDistribution, ContinuousDeploymentPolicy, RealtimeLogConfig.

### 13.4 Function ETag Duality

CloudFront Functions have a double-ETag model: `ETagDevelopment` (changes on every `UpdateFunction`) and `ETagLive` (changes only on `PublishFunction`). The handler picks the right ETag based on `Stage` query parameter. The `FunctionRecord` carries both.

---

## 14. Distribution Status State Machine

```
      CreateDistribution          UpdateDistribution          CreateInvalidation
             |                          |                            |
             v                          v                            v
      [  InProgress  ]            [  InProgress  ]            [  InProgress  ]
             |                          |                            |
             | propagation_ms           | propagation_ms             | completion_ms
             v                          v                            v
      [   Deployed   ]            [   Deployed   ]            [   Completed  ]
```

Transitions happen in background `tokio::spawn` tasks that sleep for the configured duration, then flip the status. The provider keeps `JoinHandle`s and drops them on shutdown.

Default durations are **zero**, so standard tests see `Deployed` immediately. CI integration tests that need to observe the `InProgress` state set the env var explicitly.

Behavior notes:
- `DeleteDistribution` is only allowed when `config.enabled = false`. AWS additionally requires `Status = Deployed`; Rustack enforces `Deployed` as well (matching AWS) unless `CLOUDFRONT_ALLOW_DELETE_IN_PROGRESS=true` is set, in which case we skip the status check (useful for tests that don't care about propagation).
- `UpdateDistribution` is allowed in both `InProgress` and `Deployed`; the pending transition is replaced.

---

## 15. Cross-Service Interactions

### 15.1 S3 Integration (optional, gated by `s3-integration` feature)

When a distribution's origin references an S3 bucket and has an Origin Access Control attached, AWS requires the target S3 bucket policy to grant `cloudfront.amazonaws.com` read access under an `aws:SourceArn` condition tied to the distribution ARN. IaC tools (Terraform, CDK) submit the bucket policy separately; Rustack does not modify it implicitly.

**With the feature disabled** (default), Rustack is pure-storage: it does not validate or touch S3 bucket policies. This matches what users expect for testing IaC.

**With the feature enabled**, Rustack offers an optional helper: `RustackCloudFront::attach_oac_to_bucket(&oac_id, &bucket)` that writes a canonical OAC-grant statement to the bucket's policy via `RustackS3::put_bucket_policy`. This is explicitly opt-in and not invoked automatically; it exists for convenience in scenarios where a test harness wants one-call setup.

Separately, the companion data plane (§see [rustack-cloudfront-dataplane-design.md §8.1](./rustack-cloudfront-dataplane-design.md#81-s3-origin-d0)) dispatches S3-origin requests to `rustack-s3-core` **in-process** -- a direct function call, no loopback HTTP. That in-process dispatch intentionally bypasses OAC signature verification (Rustack trusts its own S3 provider). Tests that want to assert OAC wiring should check the S3 bucket policy via the S3 management API, not rely on the data plane to reject unsigned requests.

### 15.2 Lambda@Edge

Distributions may reference Lambda function versions via `lambda_function_associations`. Rustack stores the ARNs verbatim. No invocation occurs. A future enhancement could validate that the referenced Lambda version exists when `rustack-lambda-core` is enabled, but this is Phase 4+ and not part of the initial spec.

### 15.3 WAF / Shield

WebACL ARNs are stored as opaque strings on distributions. `AssociateDistributionWebACL` / `DisassociateDistributionWebACL` simply set/clear the field. No WAF enforcement occurs (WAF is not a Rustack service).

### 15.4 ACM / IAM Certificates

`ViewerCertificate.acm_certificate_arn` and `iam_certificate_id` are stored but not validated against a real ACM or IAM resource (ACM is not a Rustack service).

---

## 16. Server Integration

### 16.1 Wiring

In `apps/rustack/src/main.rs`, add the CloudFront service router under the `cloudfront` cargo feature:

```rust
#[cfg(feature = "cloudfront")]
{
    let cloudfront_config = Arc::new(CloudFrontConfig::from_env());
    let cloudfront_provider = Arc::new(RustackCloudFront::new(cloudfront_config));
    let cloudfront_handler = RustackCloudFrontHandler::new(Arc::clone(&cloudfront_provider));
    let cloudfront_http = CloudFrontHttpService::new(cloudfront_handler);
    services.push(Box::new(CloudFrontServiceRouter::new(cloudfront_http)));
}
```

The router is registered **before** S3 (catch-all) and before any header-discriminated services. Order relative to other path-prefix services does not matter because the prefixes are disjoint.

### 16.2 Feature Flag

In `apps/rustack/Cargo.toml`:

```toml
[features]
default = ["cloudfront", ...]
cloudfront = [
    "dep:rustack-cloudfront-core",
    "dep:rustack-cloudfront-http",
    "dep:rustack-cloudfront-model",
]
s3-integration = ["cloudfront", "rustack-cloudfront-core/s3-integration"]
```

### 16.3 Health Endpoint

The gateway's health-check response automatically includes `"cloudfront": "running"` once the router is registered (no code change needed; `gateway.rs` enumerates registered service names).

### 16.4 Config Summary

| Env Var | Default | Purpose |
|---------|---------|---------|
| `CLOUDFRONT_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `CLOUDFRONT_DISTRIBUTION_PROPAGATION_MS` | `0` | InProgress→Deployed delay for Distributions |
| `CLOUDFRONT_INVALIDATION_COMPLETION_MS` | `0` | InProgress→Completed delay for Invalidations |
| `CLOUDFRONT_DETERMINISTIC_IDS` | `false` | Derive IDs from CallerReference hash |
| `CLOUDFRONT_DOMAIN_SUFFIX` | `cloudfront.net` | Suffix for generated distribution domain names |
| `CLOUDFRONT_ALLOW_DELETE_IN_PROGRESS` | `false` | Allow DeleteDistribution on non-Deployed distributions |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Account ID used in ARNs |

---

## 17. Testing Strategy

### 17.1 Unit Tests (per crate)

- `rustack-restxml`: serialize/deserialize roundtrip tests for CloudFront list patterns (`Quantity`/`Items`, `Enabled`/`Quantity`).
- `rustack-cloudfront-model`: snapshot tests for generated code (see codegen test strategy in smithy-codegen-all-services-design.md §14.2).
- `rustack-cloudfront-http`: router tests (each operation resolves to the correct enum variant; query-flag disambiguation works); request/response serialization fixtures.
- `rustack-cloudfront-core`: operation-level unit tests using an in-memory provider and hand-crafted inputs.

### 17.2 Integration Tests (`tests/` at workspace root)

Organized by resource family. Each test spins up the Rustack binary in-process and issues requests via the AWS SDK for Rust.

- `tests/cloudfront_distribution.rs`: CreateDistribution happy path, validation failures (no origins, bad target_origin_id, dangling policy ref), ETag + IfMatch, Update replace-only semantics, DeleteDistribution requires disabled, status transitions with non-zero propagation, CopyDistribution, ListDistributions pagination.
- `tests/cloudfront_invalidation.rs`: CreateInvalidation, CallerReference idempotency, status transition, ListInvalidations ordering.
- `tests/cloudfront_origin_access_control.rs`: CRUD with ETag, InUse rejection on delete.
- `tests/cloudfront_origin_access_identity.rs`: CRUD with ETag.
- `tests/cloudfront_cache_policy.rs`: CRUD, managed-policy IllegalUpdate, InUse rejection.
- `tests/cloudfront_response_headers_policy.rs`: similar.
- `tests/cloudfront_origin_request_policy.rs`: similar.
- `tests/cloudfront_key_group.rs`: CRUD + cross-check to PublicKey InUse.
- `tests/cloudfront_public_key.rs`: CRUD.
- `tests/cloudfront_function.rs`: Create, Publish, Describe at both DEVELOPMENT and LIVE stages, TestFunction canned response.
- `tests/cloudfront_realtime_log_config.rs`: CRUD.
- `tests/cloudfront_fle.rs`: Config + Profile CRUD.
- `tests/cloudfront_monitoring_subscription.rs`: single-entity lifecycle.
- `tests/cloudfront_tagging.rs`: Tag/Untag/List across Distribution, StreamingDistribution, CachePolicy.
- `tests/cloudfront_terraform.rs` (fixture-driven): run a small Terraform plan that creates an S3+OAC+Distribution and asserts `terraform apply` and `terraform destroy` complete against Rustack.

### 17.3 Protocol Fidelity Tests

Fixture directory `tests/cloudfront_fixtures/` containing real AWS XML request/response payloads (captured via `aws --debug` against a test account, scrubbed of credentials). For each fixture:
- Parse the request body into the generated input struct.
- Parse the response body into the generated output struct.
- Serialize both back; compare canonical XML (order-normalized) to the fixture.

Fixtures cover: `CreateDistribution` with SSL cert + aliases + multiple origins + Lambda@Edge assoc; `CreateInvalidation` with `/*`; `UpdateDistribution` (preserving `CallerReference`); error responses (`NoSuchDistribution`, `PreconditionFailed`).

### 17.4 Client Compatibility Matrix

For each phase, verify at least the following clients:

- `aws` CLI (the v2 Python CLI): smoke-test each covered operation.
- `aws-sdk-rust` `aws-sdk-cloudfront`: typed round-trip on every generated operation.
- `boto3`: Python smoke test.
- Terraform `aws_cloudfront_*`: `terraform plan` + `apply` + `destroy` lifecycle on a reference template per phase.
- CDK v2 `aws-cdk-lib/aws-cloudfront`: `cdk synth` + `cdk deploy --require-approval=never` + `cdk destroy` lifecycle.

### 17.5 Performance

No performance benchmarks in initial implementation. Follows project policy that benchmarks are added later with `criterion`.

---

## 18. Phased Implementation Plan

### Phase 0 — Skeleton + Distribution + Invalidation + OAC + Tags (~20 ops)

Unlocks: Terraform S3+CloudFront+OAC static-site pattern; AWS CLI invalidation automation.

- Extract `rustack-restxml` from `rustack-s3-xml`; S3 tests must continue to pass.
- Add `codegen/smithy-model/cloudfront.json` and `codegen/services/cloudfront.toml`.
- Extend codegen to emit CloudFront model crate (restXml-standard mode).
- Implement `rustack-cloudfront-http` router + dispatch.
- Implement `rustack-cloudfront-core` provider with storage, ETag/IfMatch, distribution/invalidation state machines, managed policy seeding.
- Wire `CloudFrontServiceRouter` into the gateway.
- Integration tests for distribution, invalidation, OAC, tagging.

### Phase 1 — Policies + OAI (~24 ops)

Unlocks: CDN configuration completeness for CDK `Distribution` construct.

- CachePolicy, OriginRequestPolicy, ResponseHeadersPolicy full CRUD.
- OriginAccessIdentity full CRUD.
- Managed policy seeding for all three policy kinds.
- Cross-reference `InUse` rejections on delete.
- Terraform integration tests for each policy kind.

### Phase 2 — Key Material (~12 ops)

Unlocks: CloudFront signed URLs/cookies configuration; frontend paywall IaC.

- PublicKey CRUD.
- KeyGroup CRUD with PublicKey InUse cross-checking.
- Distribution's `TrustedKeyGroups` validation against KeyGroup existence.

### Phase 3 — Functions + Realtime Logs + FLE + Monitoring + KVStore (~28 ops)

Unlocks: CloudFront Functions IaC, field-level encryption configuration, monitoring subscriptions.

- CloudFront Functions with DEVELOPMENT/LIVE staging, dual ETag.
- TestFunction canned response.
- RealtimeLogConfig CRUD.
- FLE configs + profiles CRUD.
- MonitoringSubscription (per-distribution single-entity).
- KeyValueStore CRUD (metadata only).

### Phase 4 — Everything Else (~remaining)

Unlocks: edge cases (streaming, continuous deployment, tenants, VPC origins, anycast IPs, trust stores, resource policies, connection groups/functions, ListDistributionsBy*).

- Mostly repetition of established patterns.
- Stubs for operations that IaC tools rarely emit but occasionally pop up.
- `ListDistributionsBy*` variants (by cache policy ID, by key group, etc.) as filtered iterations over the distribution store.

---

## 19. Risk Analysis

### 19.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| restXml codegen path generalizes poorly for CloudFront's Quantity/Items idiom | High | Medium | Prototype on `Distribution` first; extract `rustack-restxml::quantity_items` helper early; keep S3 byte-for-byte regression tests |
| ETag/IfMatch semantics miss edge cases that Terraform relies on | Medium | High | Dedicated unit tests for `InvalidIfMatchVersion` vs `PreconditionFailed` split; Terraform lifecycle tests in CI |
| Managed policy IDs don't match AWS's exact values | Low | Medium | Hard-code AWS's documented managed policy IDs; reference AWS docs in comments with date |
| Distribution config validation is incomplete, causing valid IaC configs to reject | Medium | High | Start permissive; only reject what is structurally impossible (e.g., DefaultCacheBehavior with a TargetOriginId that does not match any origin); expand validation based on Terraform test failures |
| Lambda@Edge and WAF association ARNs cause schema mismatches at JSON boundaries | Low | Low | Store as opaque strings; no validation |
| XML namespace mismatches between CloudFront and S3 confuse the shared parser | Medium | Low | Explicit namespace handling in `rustack-restxml::reader`; unit tests with both namespaces |
| Background state-transition tasks leak on provider drop | Medium | Low | Provider owns task handles; `Drop` sets shutdown flag and aborts handles |
| Deterministic IDs collide on repeated CallerReferences across distributions | Low | Low | Deterministic mode only for test env; real production IDs randomized |
| CloudFront Function `TestFunction` canned response mismatches what the function would have returned, breaking tests that assert function output | Medium | Low | Document the canned response clearly; gate against real-function behavior with an env var that returns `InvalidFunctionAssociation` instead, so tests can detect rather than silently pass |

### 19.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Phase 4 operations (tenants, connection functions, trust stores) have unclear semantics and ambiguous Smithy definitions | High | Low | Implement as minimal stubs that persist input and return it on read; document known-partial behavior; revisit when a real user asks |
| Scope creep into CDN data plane (users asking for actual caching) | Medium | High | Explicit non-goal in §3.2; redirect such requests to use a separate CDN test tool |
| Smithy model evolution — AWS renames operations or changes wire format | Low | Medium | `make codegen-update` + CI job detecting diffs; pin the model commit hash for reproducibility |
| Terraform AWS provider changes error-string matching | Medium | Low | Match AWS's documented error codes; error messages are best-effort |

### 19.3 Operational Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Increased Docker image size due to new crate | Low | Low | CloudFront is a single feature flag; `--no-default-features` excludes it |
| Slower test suite due to state-transition sleeps | Low | Low | Default `propagation_ms = 0`; only slow when explicitly enabled |
| Gateway routing regression (S3 paths start with `/2020-` by accident) | Low | Medium | The prefix `/2020-05-31/` is globally unique and extremely specific; router test covers S3-adjacent paths that should fall through |

---

## 20. Open Questions

- **~~Should Rustack optionally serve a 403 static response at `/_aws/cloudfront/{id}/*`?~~** *Resolved:* the companion data-plane spec ([rustack-cloudfront-dataplane-design.md](./rustack-cloudfront-dataplane-design.md)) claims `/_aws/cloudfront/{id}/*` as a real pass-through reverse-proxy prefix. Requests to disabled or missing distributions return 403/404 respectively, emitted by the data plane. When the data-plane feature is disabled, the gateway falls through to S3 and the request 404s -- which is fine because a user without the data plane is not expected to hit that URL.
- **Should `CLOUDFRONT_DOMAIN_SUFFIX` default to `cloudfront.rustack.local` instead of `cloudfront.net`?** Pro: avoids accidental prod-like domain that might confuse debugging. Con: diverges from AWS output; some SDK conformance tests assert `.cloudfront.net`. Default: `.cloudfront.net` for parity; user can override. (This env var is shared between the management and data-plane specs.)
- **Should we also emit pre-signed URL generators for signed-URL testing (data-plane helper, not server-side)?** Not in this spec's scope; could be a separate `rustack-cloudfront-signing` helper crate later.
- **Should managed policies be injected into `ListCachePolicies` output by default?** AWS does include them; Rustack should too. This will be done on provider startup via a seed step.
