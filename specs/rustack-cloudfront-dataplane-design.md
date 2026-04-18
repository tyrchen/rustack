# Rustack CloudFront Data Plane: Pass-Through Reverse Proxy Design

**Date:** 2026-04-18
**Status:** Draft / RFC
**Depends on:** [rustack-cloudfront-design.md](./rustack-cloudfront-design.md), [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md)
**Scope:** Add a minimal CloudFront **data plane** to Rustack -- an axum-based reverse proxy that accepts HTTP requests addressed to a distribution and dispatches them to the configured origin, applying a subset of the distribution's behavior-level configuration (path matching, origin selection, default root object, origin custom headers, response headers policy). Deliberately **not** a CDN: no caching, no TTLs, no compression, no Lambda@Edge/Functions execution, no signed-URL verification. The purpose is end-to-end IaC testing, not production simulation. Roughly ~1,500–2,500 lines of code across 3 phases.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Framework Choice: Why axum](#4-framework-choice-why-axum)
5. [Architecture Overview](#5-architecture-overview)
6. [Request Routing](#6-request-routing)
7. [Cache Behavior Selection](#7-cache-behavior-selection)
8. [Origin Dispatch](#8-origin-dispatch)
9. [Request and Response Transformations](#9-request-and-response-transformations)
10. [Non-Executed Features](#10-non-executed-features)
11. [Crate and Module Layout](#11-crate-and-module-layout)
12. [Gateway Integration](#12-gateway-integration)
13. [Configuration](#13-configuration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)
17. [Open Questions](#17-open-questions)

---

## 1. Executive Summary

The [CloudFront management spec](./rustack-cloudfront-design.md) defines the control plane (~90 ops, restXml, ETag/IfMatch, lifecycle simulation). It explicitly left the data plane out of scope. This spec adds a **minimal, deliberately-thin** data plane so that `curl https://{distribution-id}.cloudfront.net/path` (or the equivalent path-based URL) returns real content from the configured origin.

Key design decisions:

- **axum-based reverse proxy** -- a single `axum::Router` plugged into the existing gateway via a new `CloudFrontDataPlaneRouter` implementing `ServiceRouter`. axum composes natively with the hyper stack that powers the rest of Rustack; no second HTTP framework is introduced.
- **In-process origin dispatch is the killer feature** -- for S3 origins (the dominant case), the data plane calls `rustack-s3-core` as a direct function call. No socket, no loopback, no SigV4 re-verification. This is why adding a data plane is cheap for Rustack but prohibitively expensive for a general-purpose CDN emulator.
- **Pass-through only** -- every request hits origin. No cache, no TTL evaluation, no stale-while-revalidate. Cache policies and their IDs are respected for *reference resolution* (to pick a behavior), but their caching semantics are not enforced.
- **Dual routing strategy** -- (a) path-based URL `http://localhost:4566/_aws/cloudfront/{distribution_id}/{proxy+}` always works; (b) host-header routing on `{id}.cloudfront.net` and configured `Aliases` works when the user resolves those names to localhost (via `/etc/hosts`, `curl --resolve`, or a devbox DNS mock).
- **Behavior-aware path matching** -- CloudFront's cache-behavior precedence (ordered behaviors, path patterns with `*` glob, `DefaultCacheBehavior` catch-all) is implemented faithfully because IaC tests catch ordering bugs on real CloudFront.
- **Loud divergence signalling** -- if a request would have invoked Lambda@Edge or a CloudFront Function in real AWS, Rustack logs a `warn!` by default and continues. `CLOUDFRONT_FAIL_ON_FUNCTION=true` switches to hard-fail (500 with a clear error) so tests can detect silent behavioral divergence.
- **Phased delivery** -- D0 (S3 origin, minimum viable), D1 (custom HTTP origin + response/error policies + host routing), D2 (API Gateway and Lambda function URL origins + compression).
- **Separate crate** -- `rustack-cloudfront-dataplane`, behind a `cloudfront-dataplane` cargo feature. Disabling the feature removes axum, reqwest (D1+), and keeps the management plane lean for users who only need IaC validation.

---

## 2. Motivation

### 2.1 The End-to-End Gap

The management-plane spec unlocks `terraform apply` and `cdk deploy` for CloudFront. What it does not unlock is verifying that the **deployed configuration actually works**. A user who runs:

```bash
terraform apply  # creates S3 + OAC + Distribution
curl http://d1abcdef.cloudfront.net/index.html  # times out — no DNS, no data plane
```

finds out at deploy time in prod whether their `DefaultRootObject`, origin domain, cache behavior path patterns, and OAC wiring were correct. The management plane validates *schemas*; only a data plane validates *intent*.

Common misconfigurations that a pass-through data plane catches, but schema-only testing does not:

- `Origin.DomainName` wrong (typo in bucket name, wrong region suffix).
- `TargetOriginId` in a cache behavior does not match any `Origin.Id`.
- `DefaultRootObject` missing, so `/` returns `XML/ListBucket` from S3 instead of `index.html`.
- `Origins[].OriginPath` concatenation produces the wrong key (double slash, missing slash).
- Cache behavior ordering wrong -- a permissive wildcard shadows a stricter path pattern.
- `CustomErrorResponses` references a path that does not exist in the origin bucket.
- OAC attached to the distribution but bucket policy does not grant read -- S3 returns 403 and CloudFront bubbles it up.

These are the boring, load-bearing bugs that break static sites in production. The data plane catches them locally.

### 2.2 What "Minimal" Means Here

The data plane is *not* a CDN. It does not:

- Evaluate `MinTTL` / `MaxTTL` / `DefaultTTL` -- nothing is cached.
- Apply compression based on `Compress` flag -- the origin's compression, if any, is passed through.
- Implement request coalescing, origin shield, or regional caching.
- Enforce rate limits, WAF rules, Shield DDoS protection, or geo restrictions.
- Terminate TLS on custom domains -- Rustack is HTTP-only locally.
- Provide anycast IP addresses or edge location simulation.

It *does* route requests, select origins, apply simple transformations, and return responses. That is the 80/20 cut.

### 2.3 Value Proposition

| Benefit | Who it unblocks |
|---------|-----------------|
| `curl` a distribution URL and see real S3 content | Everyone testing static-site IaC |
| Playwright/Cypress integration tests against a local `cloudfront.net` domain | Frontend teams with E2E test suites |
| Detect `DefaultRootObject` / path-pattern / OAC misconfigurations locally | Platform engineers, DevOps |
| Test cache-invalidation workflows end-to-end (invalidate + refetch from origin) | CI/CD pipelines doing deploy+bust |
| Test CloudFront → API Gateway → Lambda chains locally (D2) | Serverless API developers |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **S3 origin pass-through** (D0) -- distributions pointing at S3 serve GET/HEAD requests by calling `rustack-s3-core` in-process.
2. **Custom HTTP origin pass-through** (D1) -- distributions pointing at an arbitrary HTTP URL reverse-proxy via `reqwest`.
3. **Cache-behavior path matching** -- ordered behaviors with path patterns (exact, wildcard `*`, prefix) selected correctly, with `DefaultCacheBehavior` as catch-all.
4. **`DefaultRootObject` rewrite** -- requests to `/` are rewritten to `/{default_root_object}` before behavior matching.
5. **`OriginPath` prepend** -- requests to `/foo` with `OriginPath=/v1` are sent to origin as `/v1/foo`.
6. **Origin custom headers** -- `Origin.CustomHeaders` are added to the upstream request.
7. **Response Headers Policy** (D1) -- if referenced, the policy's `CustomHeadersConfig`, `CorsConfig`, `SecurityHeadersConfig` headers are added to the downstream response.
8. **Custom Error Responses** (D1) -- if the origin returns a status listed in `CustomErrorResponses`, serve the configured error document with the configured `ResponseCode`.
9. **Dual routing** -- path-based (`/_aws/cloudfront/{id}/...`) always works; host-header routing works for `.cloudfront.net` suffix and configured `Aliases`.
10. **Disabled distribution handling** -- `Enabled: false` returns 403 with a body matching CloudFront's shape.
11. **HEAD and GET support** -- methods CloudFront allows in `AllowedMethods` are honored. Others return 405.
12. **API Gateway v2 origin** (D2) -- origin URL matching the APIGW v2 execution pattern dispatches in-process to `rustack-apigatewayv2-core`.
13. **Lambda function URL origin** (D2) -- origin URL matching the Lambda function URL pattern dispatches in-process to `rustack-lambda-core`.
14. **Loud divergence** -- Lambda@Edge and CloudFront Function associations log a `warn!` by default; `CLOUDFRONT_FAIL_ON_FUNCTION=true` hard-fails with 500 and a clear error code.
15. **Feature-flagged** -- entire data plane lives behind a cargo feature; building without it keeps the binary lean.

### 3.2 Non-Goals

1. **No caching** -- `MinTTL`, `MaxTTL`, `DefaultTTL`, `Cache-Control` header evaluation, conditional GET handling (`If-None-Match`, `If-Modified-Since` pass through to origin, but Rustack does not interpret them itself). This is the single biggest scope reduction; it is what makes the data plane tractable.
2. **No Lambda@Edge execution** -- associations are stored on distributions; a request that would have triggered one logs a warning (or hard-fails under `FAIL_ON_FUNCTION`).
3. **No CloudFront Functions execution** -- same posture.
4. **No signed URL / signed cookie verification** -- public keys are stored by the management plane; the data plane does not verify request signatures. `TrustedKeyGroups` and `TrustedSigners` are ignored at request time.
5. **No geo restrictions** -- `Restrictions.GeoRestriction` is stored; the data plane does not look up client country and therefore does not enforce allow/deny lists.
6. **No viewer protocol redirection** -- `ViewerProtocolPolicy: redirect-to-https` and `https-only` log a warning and serve the request over HTTP anyway, because Rustack is HTTP-only locally. Enforcing `https-only` would break every local test.
7. **No compression** (D0-D1) -- `Compress: true` does not gzip/br-encode responses that the origin did not. (D2 optionally adds Brotli/gzip for custom HTTP origins.)
8. **No request/response streaming optimizations** -- bodies are buffered up to a configurable cap (default 64 MiB), then fail with 413. Real CloudFront streams; Rustack buffers for simplicity. Raising the cap is safe for local testing.
9. **No WAF / Shield / DDoS enforcement** -- WebACL ARNs are ignored at request time.
10. **No OriginGroup failover semantics** (D0) -- accept the configuration, treat as primary-only. Add failover in D2 if demand materializes.
11. **No TLS termination** -- Rustack runs HTTP. TLS on CloudFront aliases is a non-goal.
12. **No per-request metrics / real-time log delivery** -- `tracing` logs the request. `RealtimeLogConfig` is stored by the management plane; the data plane does not emit to it.
13. **No staging distribution / continuous deployment traffic shifting** -- staging configs are stored but all requests go to the primary.
14. **No `FieldLevelEncryption`** -- FLE configs are stored, never applied. No request bodies are transformed.

### 3.3 Explicit Posture on Signals That Should Fail

Rustack silently diverging from AWS is the worst failure mode -- tests pass locally and break in prod. To mitigate:

- Lambda@Edge/Function triggers log `warn!` by default; `FAIL_ON_FUNCTION` env var upgrades to hard-fail.
- Signed-URL-required behaviors log `warn!` once per distribution ID at startup and once per request in debug mode.
- Origin failover config logs `warn!` when the primary fails (in D0-D1; D2+ implements actual failover).
- `https-only` viewer protocol policy logs `warn!` on every matched request (rate-limited to once per minute per distribution).

The goal: the user *sees* divergence in their test output. Silent passes are the enemy.

---

## 4. Framework Choice: Why axum

The design discussion considered Pingora (Cloudflare's Rust reverse proxy framework) as an alternative. The short version of why axum wins for this specific use case:

| Concern | axum | Pingora |
|---------|------|---------|
| Integrates with the existing `hyper::service::Service`-based gateway | Yes (native hyper) | No (separate HTTP stack, separate server loop) |
| In-process origin dispatch (function call into `rustack-s3-core`) | Natural; handlers are just async fns | Fights the framework; `Peer` model assumes network upstreams |
| Binary/compile-time footprint | Small; already transitively pulled in by several deps | Large; adds boringssl, quiche (H/3), separate connection pool |
| Caching, H/2 multiplexing, connection pooling | Not provided (which is fine; we don't want them) | Provided, unused, creates scope-creep pressure |
| Consistency with rest of Rustack's HTTP code | Matches (hyper + tower idioms throughout) | Introduces a second HTTP idiom |
| Maturity | Stable, widely deployed in Rust ecosystem | OSS since 2024; still evolving |

Pingora is excellent at being a Cloudflare-scale edge proxy. That is not what this subsystem is. The framework's strengths (pooling, multiplexing, graceful reloads, health checks, filter pipeline) target production CDN concerns that are explicit non-goals here; using Pingora for this use case is a jet engine on a tricycle.

axum's `Router`, `extract::Path`, `extract::Host`, `extract::Request`, and the `tower::Service` adapter are exactly the right level of abstraction. The entire data plane is roughly one router, two handlers (path-based and host-based), and an origin dispatch switch.

If Rustack later adds a subsystem that genuinely needs proxy-grade features -- for instance, emulating ALB with realistic backpressure and connection pooling -- Pingora becomes defensible for *that* subsystem as a separate listener on its own port. For CloudFront's data plane today, it is the wrong tool.

---

## 5. Architecture Overview

```
       curl / browser / Playwright / aws-sdk-rust
                       |
                       | http://localhost:4566/_aws/cloudfront/{id}/...
                       | OR
                       | Host: {id}.cloudfront.net
                       v
              +---------------------+
              |   Gateway Router    |  GatewayService
              +--------+------------+
                       |
            (matches path prefix /_aws/cloudfront/
             OR Host header ending in .cloudfront.net
             OR Host header matching a distribution alias)
                       |
                       v
              +---------------------------------+
              | CloudFrontDataPlaneRouter       |
              | (ServiceRouter impl)            |
              +---------+-----------------------+
                        |
                        v
              +---------------------+
              |   axum::Router      |  /router/{distribution_id}/*path
              +---------+-----------+
                        |
                        v
              +---------------------+
              | DataPlaneHandler    |
              |                     |
              | 1. Resolve distrib  |---> reads from Arc<RustackCloudFront> store
              | 2. Apply DefRoot    |
              | 3. Match behavior   |---> cache-behavior precedence
              | 4. Resolve origin   |
              | 5. Apply origin cfg |
              | 6. Dispatch         |
              | 7. Apply resp cfg   |
              | 8. Return response  |
              +--+---------+--------+
                 |         |
         S3      |         |        HTTP / APIGW v2 / Lambda URL
         origin  |         |        origin
                 v         v
        +----------+   +-------------------+
        | S3 Core  |   | reqwest (D1)      |
        | (in-proc)|   | APIGW Core (D2)   |
        +----------+   | Lambda Core (D2)  |
                       +-------------------+
```

The data plane is **stateless**: it reads distribution, cache policy, origin access control, response headers policy, and custom error response configuration from the management-plane store on every request. Updates to the management plane are reflected on the next data-plane request with no invalidation step.

---

## 6. Request Routing

Two routing strategies, both supported:

### 6.1 Path-Based (always works)

```
GET http://localhost:4566/_aws/cloudfront/E1ABCDEF123456/index.html
                         |_________________________________|
                         data plane prefix + distribution ID + object path
```

The `ServiceRouter::matches` check:

```rust
fn matches(&self, req: &http::Request<Incoming>) -> bool {
    let path = req.uri().path();
    // Path-based: /_aws/cloudfront/{id}/...
    if let Some(rest) = path.strip_prefix("/_aws/cloudfront/") {
        if let Some(slash) = rest.find('/') {
            let id = &rest[..slash];
            // Valid distribution IDs are 14 chars, uppercase alphanumeric,
            // starting with 'E'. Narrow match reduces accidental collisions.
            if id.len() == 14 && id.starts_with('E')
                && id.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            {
                return true;
            }
        }
        return false;
    }

    // Host-based: see 6.2
    if let Some(host) = req.headers().get(http::header::HOST).and_then(|v| v.to_str().ok()) {
        return host_is_cloudfront_distribution(host, &self.provider);
    }

    false
}
```

Path-based URLs always resolve because they do not depend on DNS.

### 6.2 Host-Based (works when DNS is set up)

```
GET http://d1abcdef123456.cloudfront.net/index.html
Host: d1abcdef123456.cloudfront.net
```

The `host_is_cloudfront_distribution` check accepts a host if either:

1. **`{lowercased-id}.{CLOUDFRONT_DOMAIN_SUFFIX}` pattern** -- extract the leading subdomain, uppercase, confirm it corresponds to a known distribution. Default suffix is `cloudfront.net`.
2. **Match against any distribution's `Aliases.Items`** -- a linear scan over the distributions store; acceptable at Rustack's scale (dozens of distributions, not millions).

Users enable host-based routing by adding entries to `/etc/hosts` or using `curl --resolve`:

```
127.0.0.1 d1abcdef123456.cloudfront.net
127.0.0.1 www.example.com
```

Or with curl:

```
curl --resolve www.example.com:4566:127.0.0.1 http://www.example.com:4566/
```

This is documented as the "realistic test" mode. CI pipelines that need stable host-based routing can script the `/etc/hosts` append or run against a container that sets it up.

### 6.3 Order of Registration in the Gateway

The data plane router is registered **before** the CloudFront management router and **before** the S3 catch-all:

```rust
services.push(Box::new(CloudFrontDataPlaneRouter::new(...)));   // <-- NEW
services.push(Box::new(CloudFrontServiceRouter::new(...)));     // management
// ... other path-prefix services ...
// ... header-based services ...
services.push(Box::new(S3ServiceRouter::new(...)));             // catch-all
```

The path prefixes `/_aws/cloudfront/` (data plane) and `/2020-05-31/` (management) are disjoint, so they do not conflict with each other. Host-based data plane matching is by Host header, which management never touches.

---

## 7. Cache Behavior Selection

CloudFront distributions have an ordered list of `CacheBehaviors` plus one `DefaultCacheBehavior`. For each incoming request path, the behavior is selected by:

1. Iterate `CacheBehaviors` in order (array index order as stored).
2. For each, test the `PathPattern` against the request path (after `DefaultRootObject` rewrite; see §9.1).
3. The first match wins.
4. If none match, `DefaultCacheBehavior` is used (it has no `PathPattern`; it is the catch-all).

### 7.1 Path Pattern Syntax

CloudFront's `PathPattern` supports:

- **Exact match**: `/robots.txt` matches only `/robots.txt`.
- **Suffix wildcard**: `/images/*` matches `/images/`, `/images/foo`, `/images/a/b/c`.
- **Middle wildcard**: `*.jpg` matches any path ending in `.jpg`.
- **Character wildcards**: `?` matches a single character (rarely used; implement for correctness).
- **Case sensitivity**: paths are case-sensitive.
- **No regex**: `*` and `?` are the only special characters. `.`, `/`, etc. are literal.

### 7.2 Implementation

```rust
/// Match a CloudFront path pattern against a request path.
///
/// `*` matches any sequence of characters; `?` matches exactly one character.
/// All other characters are literal. Matching is case-sensitive.
pub fn matches_pattern(pattern: &str, path: &str) -> bool {
    // Translate to a simple glob matcher. For the small number of distributions
    // and behaviors per distribution (typically <20), a compiled regex is
    // unnecessary overhead.
    glob_match(pattern.as_bytes(), path.as_bytes())
}

fn glob_match(pattern: &[u8], s: &[u8]) -> bool {
    // Standard iterative glob match with *-backtracking. No allocations.
    let (mut p, mut i) = (0usize, 0usize);
    let (mut last_star, mut last_match) = (None, 0usize);

    while i < s.len() {
        match pattern.get(p) {
            Some(&b'*') => { last_star = Some(p); last_match = i; p += 1; }
            Some(&b'?') => { p += 1; i += 1; }
            Some(&c) if c == s[i] => { p += 1; i += 1; }
            _ => match last_star {
                Some(sp) => { p = sp + 1; last_match += 1; i = last_match; }
                None => return false,
            },
        }
    }
    while pattern.get(p) == Some(&b'*') { p += 1; }
    p == pattern.len()
}
```

Unit tests cover: exact match, `*` suffix, `*.jpg`, nested paths, `?` single-char, non-matches, empty pattern against empty path, patterns with literal `.`.

### 7.3 Selection Output

Behavior selection yields:

- The resolved `Origin` record (looked up by `TargetOriginId`).
- The `CachePolicyId` (used for origin cache-key construction hints; today only for logging, not enforcement).
- The `OriginRequestPolicyId` (referenced, not enforced).
- The `ResponseHeadersPolicyId` (applied in §9.4).
- The `AllowedMethods` set (used to 405 unsupported methods).
- The `LambdaFunctionAssociations` and `FunctionAssociations` (logged/failed per §10).

---

## 8. Origin Dispatch

After behavior selection, the resolved `Origin` determines dispatch. The data plane inspects `Origin.DomainName` (and optionally `S3OriginConfig` / `CustomOriginConfig`) to pick the dispatch path.

### 8.1 S3 Origin (D0)

An origin is an S3 origin if any of the following is true:

- `S3OriginConfig` is present.
- `DomainName` matches `{bucket}.s3.{region}.amazonaws.com` or `{bucket}.s3.amazonaws.com`.
- `DomainName` matches `{bucket}.s3-website-{region}.amazonaws.com` (S3 website endpoint -- treat same as S3 origin for Rustack's purposes).

Parse the bucket name from the domain and dispatch in-process:

```rust
pub async fn dispatch_s3_origin(
    s3: &Arc<RustackS3>,
    bucket: &str,
    origin_path: &str,
    request_path: &str,
    method: &http::Method,
    headers: &http::HeaderMap,
    origin_custom_headers: &[CustomHeader],
) -> Result<http::Response<Bytes>, DataPlaneError> {
    // Combine OriginPath + request path, ensuring exactly one '/' separator.
    let key = format!(
        "{}{}",
        origin_path.trim_end_matches('/'),
        request_path,
    );
    let key = key.trim_start_matches('/');

    match *method {
        http::Method::GET => s3_get(s3, bucket, key, headers, origin_custom_headers).await,
        http::Method::HEAD => s3_head(s3, bucket, key, headers).await,
        _ => Err(DataPlaneError::MethodNotAllowed),
    }
}
```

The `s3_get` and `s3_head` helpers invoke `RustackS3::get_object` / `head_object` directly, translating the S3 response into an HTTP response. Headers surfaced to the client:

- `Content-Type` (from S3 object metadata)
- `Content-Length`
- `ETag`
- `Last-Modified`
- `Cache-Control` (if set on the object)
- `x-amz-meta-*` headers (stripped by default; configurable via `CLOUDFRONT_FORWARD_USER_METADATA=true`)
- Additional headers from `ResponseHeadersPolicy` (see §9.4).

On S3 errors, translate:

- `NoSuchKey` → 404 with a minimal HTML body (configurable via `CustomErrorResponses`).
- `NoSuchBucket` → 502 (origin configuration error).
- `AccessDenied` → 403.

### 8.2 Custom HTTP Origin (D1)

An origin is a custom HTTP origin if it has `CustomOriginConfig` set. Dispatch via `reqwest`:

```rust
pub async fn dispatch_http_origin(
    client: &reqwest::Client,
    origin: &ResolvedOrigin,
    request_path: &str,
    method: &http::Method,
    headers: &http::HeaderMap,
    body: Bytes,
) -> Result<http::Response<Bytes>, DataPlaneError> {
    let scheme = match origin.custom_config.origin_protocol_policy.as_str() {
        "http-only" | "match-viewer" => "http",
        "https-only" => "https",
        _ => "https",
    };
    let port = match scheme {
        "http" => origin.custom_config.http_port.unwrap_or(80),
        "https" => origin.custom_config.https_port.unwrap_or(443),
        _ => 443,
    };
    let url = format!(
        "{scheme}://{}:{port}{}{request_path}",
        origin.domain_name,
        origin.origin_path,
    );
    let upstream_req = client
        .request(method.clone(), &url)
        .headers(translate_headers(headers, &origin.custom_headers))
        .body(body)
        .build()?;
    let resp = client.execute(upstream_req).await?;
    translate_response(resp).await
}
```

Timeouts: `origin.custom_config.origin_read_timeout` (default 30s) and `origin_connection_timeout` (default 10s). Enforced at the reqwest client level, not per-request (acceptable simplification).

### 8.3 Origin Groups (non-goal in D0-D1)

`OriginGroups` define primary/failover pairs. In D0-D1, the data plane reads the primary and ignores failover. Accessing a distribution whose behavior points to an origin group logs a `warn!` and proceeds with the primary.

Full failover (D2+): on primary returning a status in `OriginGroup.FailoverCriteria.StatusCodes`, retry against the secondary. This is straightforward once the dispatch abstraction is in place.

### 8.4 API Gateway v2 Origin (D2)

Detect by `DomainName` matching `{apiId}.execute-api.{region}.amazonaws.com`. Dispatch in-process to `rustack-apigatewayv2-core::execution::handle_execution`, reusing the same entry point as the APIGW v2 execution router.

### 8.5 Lambda Function URL Origin (D2)

Detect by `DomainName` matching `{urlId}.lambda-url.{region}.on.aws`. Dispatch in-process to `rustack-lambda-core` by invoking the function URL, constructing a Lambda function URL event payload from the HTTP request, and translating the Lambda response back to HTTP.

---

## 9. Request and Response Transformations

### 9.1 DefaultRootObject Rewrite

If the request path is exactly `/` and `DistributionConfig.DefaultRootObject` is set, rewrite to `/{default_root_object}` **before** behavior matching and origin dispatch.

```rust
fn apply_default_root_object(path: &str, default: &str) -> Cow<'_, str> {
    if path == "/" && !default.is_empty() {
        Cow::Owned(format!("/{}", default.trim_start_matches('/')))
    } else {
        Cow::Borrowed(path)
    }
}
```

Edge case: `/` with an empty `DefaultRootObject` falls through to origin as `/`, which for S3 returns a `ListBucket` XML response. This matches AWS behavior.

### 9.2 OriginPath Prepend

`Origin.OriginPath` is a directory prefix prepended to every request. For example, `OriginPath=/v1` and request `/api/users` means the origin sees `/v1/api/users`. Implementation is a careful string concat that avoids double slashes or missing slashes (see §8.1).

### 9.3 Origin Custom Headers

`Origin.CustomHeaders` is a list of `(HeaderName, HeaderValue)` pairs added to the upstream request. If the client already set the header, CloudFront overwrites it with the origin custom header (documented AWS behavior).

```rust
fn apply_custom_headers(
    mut upstream_headers: http::HeaderMap,
    custom: &[CustomHeader],
) -> http::HeaderMap {
    for h in custom {
        if let (Ok(name), Ok(val)) = (
            http::HeaderName::from_bytes(h.header_name.as_bytes()),
            http::HeaderValue::from_str(&h.header_value),
        ) {
            upstream_headers.insert(name, val);
        }
    }
    upstream_headers
}
```

### 9.4 Response Headers Policy (D1)

If the selected behavior has `ResponseHeadersPolicyId`, the referenced policy's headers are applied to the downstream response:

- `CorsConfig` -- `Access-Control-*` headers when the request has `Origin`.
- `SecurityHeadersConfig` -- `Strict-Transport-Security`, `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`, `Content-Security-Policy`.
- `CustomHeadersConfig` -- user-defined headers.
- `ServerTimingHeadersConfig` -- adds `Server-Timing` with a synthetic `origin; dur=NN` entry.
- `RemoveHeadersConfig` -- strips listed headers from the upstream response.

Policy resolution uses the management-plane store; managed policies (`CORS-With-Preflight`, `SecurityHeadersPolicy`, etc.) are pre-seeded.

### 9.5 Custom Error Responses (D1)

If the upstream response status matches an entry in `DistributionConfig.CustomErrorResponses.Items`:

1. Fetch the configured `ResponsePagePath` from the origin (the origin of the *default* behavior, not the matched behavior -- matching AWS docs).
2. Return the fetched content with `ResponseCode` (overriding the original) and `Cache-Control: max-age={ErrorCachingMinTTL}` as a header (honest about TTL but we do not actually cache).
3. If the error-page fetch itself fails, return the original upstream response.

### 9.6 Header Normalization on Pass-Through

Inbound headers the data plane strips before forwarding to origin (following real CloudFront's documented behavior):

- `Connection`, `Keep-Alive`, `Proxy-Authenticate`, `Proxy-Authorization`, `TE`, `Trailers`, `Transfer-Encoding`, `Upgrade` (hop-by-hop headers).
- `Expect` (CloudFront does not forward).
- `X-Amz-Cf-*` (CloudFront-reserved headers; stripped).

Outbound headers added on response:

- `X-Cache: Miss from rustack-cloudfront` (informational; always a miss because we do not cache).
- `X-Amz-Cf-Id: {random uuid-like token}` (tests that assert on request IDs need one).
- `Via: 1.1 rustack.cloudfront.net (CloudFront)`.

---

## 10. Non-Executed Features

The data plane deliberately does not execute several features. Each has a defined non-execution behavior:

| Feature | Default Behavior | `FAIL_ON_FUNCTION` Behavior |
|---------|-----------------|------------------------------|
| `LambdaFunctionAssociations` on a behavior | `warn!` once per request, pass through | 500 with `X-Amzn-Errortype: LambdaEdgeExecutionSkipped` |
| `FunctionAssociations` on a behavior | Same | 500 with `X-Amzn-Errortype: CloudFrontFunctionExecutionSkipped` |
| `TrustedKeyGroups` / `TrustedSigners` on a behavior | `warn!` once per distribution at first hit, pass through | 403 with `SignedUrlRequired` |
| `Restrictions.GeoRestriction` | `warn!` once per distribution, pass through | Pass through (no geo data available to enforce) |
| `WebACLId` set on distribution | `warn!` once per distribution, pass through | Pass through (WAF is not a Rustack service) |
| `ViewerProtocolPolicy: https-only` | `warn!` rate-limited, serve over HTTP | 403 with `ViewerProtocolPolicyViolation` |
| `ViewerProtocolPolicy: redirect-to-https` | `warn!` rate-limited, serve over HTTP | 301 to `https://{host}{path}` (only useful if user also has an HTTPS listener) |
| `FieldLevelEncryptionId` on a behavior | `warn!` once, pass through | 500 with `FieldLevelEncryptionSkipped` |

The `FAIL_ON_FUNCTION` name is slightly misleading (it covers more than functions) but kept for consistency with the env-var naming used in the main CloudFront spec.

---

## 11. Crate and Module Layout

### 11.1 New Crate: `rustack-cloudfront-dataplane`

A separate crate (not a module in `rustack-cloudfront-core`) for two reasons:

1. **Feature isolation** -- the `cloudfront-dataplane` cargo feature gates axum, reqwest, tower, and the (Phase D2) `rustack-apigatewayv2-core` / `rustack-lambda-core` dependencies. Users who only want the management plane pay none of this.
2. **Dependency direction** -- the data plane depends on the management plane (for distribution lookups), not the other way around. A separate crate makes that unidirectional relationship explicit.

```
crates/rustack-cloudfront-dataplane/
├── Cargo.toml
└── src/
    ├── lib.rs               # DataPlane struct, builder, public API
    ├── config.rs            # DataPlaneConfig (env-loaded)
    ├── router.rs            # axum::Router construction + route handlers
    ├── host.rs              # Host header parsing and distribution resolution
    ├── behavior.rs          # Cache behavior selection + PathPattern glob matcher
    ├── dispatch/
    │   ├── mod.rs
    │   ├── s3.rs            # In-process S3 origin dispatch
    │   ├── http.rs          # reqwest-based custom HTTP origin (D1)
    │   ├── apigw.rs         # In-process APIGW v2 origin (D2)
    │   └── lambda_url.rs    # In-process Lambda function URL origin (D2)
    ├── transform/
    │   ├── mod.rs
    │   ├── headers.rs       # Header stripping, custom header application, Via/X-Cache
    │   ├── default_root.rs  # DefaultRootObject rewrite
    │   ├── response_policy.rs   # ResponseHeadersPolicy application
    │   └── error_response.rs    # CustomErrorResponses handling
    ├── divergence.rs        # warn!/fail logic for non-executed features
    └── error.rs             # DataPlaneError + conversion to HTTP response
```

### 11.2 Dependencies

```toml
[package]
name = "rustack-cloudfront-dataplane"
version = { workspace = true }
edition = "2024"

[dependencies]
rustack-cloudfront-core = { workspace = true }
rustack-s3-core = { workspace = true }
axum = "0.8"
tower = { version = "0.5", features = ["util"] }
tower-service = "0.3"
hyper = { workspace = true }
http = { workspace = true }
http-body-util = { workspace = true }
bytes = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }

# Optional dependencies gated by phase features
reqwest = { version = "0.12", features = ["rustls-tls"], optional = true }
rustack-apigatewayv2-core = { workspace = true, optional = true }
rustack-lambda-core = { workspace = true, optional = true }

[features]
default = []
http-origin = ["dep:reqwest"]                                  # D1
apigw-origin = ["dep:rustack-apigatewayv2-core"]               # D2
lambda-url-origin = ["dep:rustack-lambda-core"]                # D2
all-origins = ["http-origin", "apigw-origin", "lambda-url-origin"]
```

### 11.3 Public API Surface

```rust
pub struct DataPlane {
    inner: Arc<DataPlaneInner>,
}

impl DataPlane {
    pub fn builder() -> DataPlaneBuilder { DataPlaneBuilder::default() }

    /// Build the axum router for this data plane. Handler wires it into
    /// the gateway via a `ServiceRouter`.
    pub fn router(&self) -> axum::Router { ... }
}

#[derive(Default)]
pub struct DataPlaneBuilder {
    cloudfront: Option<Arc<RustackCloudFront>>,
    s3: Option<Arc<RustackS3>>,
    #[cfg(feature = "apigw-origin")]
    apigw: Option<Arc<RustackApiGatewayV2>>,
    #[cfg(feature = "lambda-url-origin")]
    lambda: Option<Arc<RustackLambda>>,
    config: DataPlaneConfig,
}

impl DataPlaneBuilder {
    pub fn cloudfront(mut self, p: Arc<RustackCloudFront>) -> Self { ... }
    pub fn s3(mut self, p: Arc<RustackS3>) -> Self { ... }
    pub fn config(mut self, c: DataPlaneConfig) -> Self { ... }
    pub fn build(self) -> Result<DataPlane, BuildError> { ... }
}
```

---

## 12. Gateway Integration

### 12.1 New ServiceRouter

Add to `apps/rustack/src/service.rs`:

```rust
#[cfg(feature = "cloudfront-dataplane")]
mod cloudfront_dataplane_router {
    use std::{convert::Infallible, future::Future, pin::Pin};
    use http_body_util::BodyExt;
    use hyper::{body::Incoming, service::Service};
    use rustack_cloudfront_dataplane::DataPlane;
    use tower::ServiceExt;

    use super::{GatewayBody, ServiceRouter};

    pub struct CloudFrontDataPlaneRouter {
        inner: DataPlane,
        // Pre-built axum router (avoids rebuilding per request).
        service: tower::util::BoxCloneService<
            http::Request<Incoming>,
            http::Response<axum::body::Body>,
            Infallible,
        >,
    }

    impl CloudFrontDataPlaneRouter {
        pub fn new(inner: DataPlane) -> Self {
            let service = inner.router().into_service().boxed_clone();
            Self { inner, service }
        }
    }

    impl ServiceRouter for CloudFrontDataPlaneRouter {
        fn name(&self) -> &'static str { "cloudfront-dataplane" }

        fn matches(&self, req: &http::Request<Incoming>) -> bool {
            let path = req.uri().path();
            if path.starts_with("/_aws/cloudfront/") {
                return true;
            }
            // Host-based routing
            if let Some(host) = req.headers().get(http::header::HOST).and_then(|v| v.to_str().ok()) {
                return self.inner.matches_host(host);
            }
            false
        }

        fn call(&self, req: http::Request<Incoming>)
            -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
        {
            let svc = self.service.clone();
            Box::pin(async move {
                let resp = svc.oneshot(req).await.unwrap_or_else(|e| match e {});
                Ok(resp.map(|b| b.map_err(|e| std::io::Error::other(e)).boxed()))
            })
        }
    }
}
```

### 12.2 Wiring in `main.rs`

```rust
#[cfg(all(feature = "cloudfront", feature = "cloudfront-dataplane"))]
{
    let dataplane = DataPlane::builder()
        .cloudfront(Arc::clone(&cloudfront_provider))
        .s3(Arc::clone(&s3_provider))
        .config(DataPlaneConfig::from_env())
        .build()?;
    services.push(Box::new(CloudFrontDataPlaneRouter::new(dataplane)));
}
```

Registration order: **before** management and **before** S3 catch-all.

### 12.3 Feature Flags

In `apps/rustack/Cargo.toml`:

```toml
[features]
default = ["cloudfront", "cloudfront-dataplane-s3", ...]
cloudfront-dataplane-s3 = [
    "cloudfront",
    "dep:rustack-cloudfront-dataplane",
]
cloudfront-dataplane-http = [
    "cloudfront-dataplane-s3",
    "rustack-cloudfront-dataplane/http-origin",
]
cloudfront-dataplane-apigw = [
    "cloudfront-dataplane-s3",
    "rustack-cloudfront-dataplane/apigw-origin",
    "apigatewayv2",
]
cloudfront-dataplane-lambda = [
    "cloudfront-dataplane-s3",
    "rustack-cloudfront-dataplane/lambda-url-origin",
    "lambda",
]
cloudfront-dataplane-full = [
    "cloudfront-dataplane-s3",
    "cloudfront-dataplane-http",
    "cloudfront-dataplane-apigw",
    "cloudfront-dataplane-lambda",
]
```

---

## 13. Configuration

| Env Var | Default | Purpose |
|---------|---------|---------|
| `CLOUDFRONT_DATAPLANE_ENABLED` | `true` when feature compiled | Runtime kill switch |
| `CLOUDFRONT_DOMAIN_SUFFIX` | `cloudfront.net` | Host suffix for host-based routing (shared with management spec) |
| `CLOUDFRONT_FAIL_ON_FUNCTION` | `false` | Hard-fail when Lambda@Edge / Function / signed URL would have been required |
| `CLOUDFRONT_FORWARD_USER_METADATA` | `false` | Include `x-amz-meta-*` headers on the response for S3 origins |
| `CLOUDFRONT_MAX_UPSTREAM_BODY_BYTES` | `67108864` (64 MiB) | Maximum response body size to buffer before 413 |
| `CLOUDFRONT_HTTP_ORIGIN_TIMEOUT_MS` | `30000` | Default custom HTTP origin read timeout |
| `CLOUDFRONT_DIVERGENCE_LOG_INTERVAL_MS` | `60000` | Minimum interval between duplicate divergence warnings per distribution |

All configuration is read at server startup via `DataPlaneConfig::from_env()`. No hot-reload.

---

## 14. Testing Strategy

### 14.1 Unit Tests (`rustack-cloudfront-dataplane`)

- `behavior::tests` -- cache-behavior selection: exact, wildcard, `?`, nested, non-match, default fallback. Property test: first-match-wins over an arbitrary behavior list.
- `host::tests` -- host parsing: `{id}.cloudfront.net`, custom suffixes, aliases, case handling, invalid hosts.
- `transform::headers::tests` -- hop-by-hop stripping, `X-Amz-Cf-*` stripping, custom header overwrite.
- `transform::default_root::tests` -- `/` rewrite, empty default, non-root paths.
- `dispatch::s3::tests` -- OriginPath concat with/without leading slash, bucket extraction from various DomainName shapes.
- `divergence::tests` -- rate-limited warning emission, FAIL_ON_FUNCTION short-circuit paths.

### 14.2 Integration Tests (`tests/cloudfront_dataplane_*.rs`)

Each test spins up Rustack in-process, creates resources via the AWS SDK, and hits the data plane with `reqwest`.

- `tests/cloudfront_dataplane_s3.rs`:
  - Create bucket, put object, create distribution with S3 origin + OAC → GET via path-based URL → assert body + headers.
  - Same via host-based URL (using `reqwest::Client::builder().resolve(...)`).
  - `DefaultRootObject` rewrite: GET `/` returns `index.html`.
  - `OriginPath` prepend: GET `/foo` hits `s3://bucket/prefix/foo`.
  - Multiple cache behaviors: assert the right one matches.
  - Disabled distribution → 403.
  - Missing distribution → 404 with CloudFront-shaped error body.
  - Method not in AllowedMethods → 405.

- `tests/cloudfront_dataplane_http.rs` (D1):
  - Start a local HTTP test server (wiremock or a small hyper handler).
  - Create distribution with a custom HTTP origin pointing at it.
  - GET through the data plane → assert pass-through.
  - `CustomErrorResponses` on 404 → assert configured page served.

- `tests/cloudfront_dataplane_response_policy.rs` (D1):
  - Create `ResponseHeadersPolicy` with `SecurityHeadersConfig`.
  - Attach to a behavior; assert response has `X-Content-Type-Options: nosniff` etc.
  - Managed policy reference.

- `tests/cloudfront_dataplane_divergence.rs`:
  - Distribution with a `LambdaFunctionAssociation` → assert `warn!` log + pass-through.
  - Same with `CLOUDFRONT_FAIL_ON_FUNCTION=true` → assert 500 + `X-Amzn-Errortype`.
  - Distribution with `TrustedKeyGroups` → same.

- `tests/cloudfront_dataplane_apigw.rs` (D2):
  - Create an APIGW v2 HTTP API + Lambda integration.
  - Create a distribution with the APIGW origin.
  - GET through the data plane → assert the Lambda was invoked in-process.

- `tests/cloudfront_dataplane_terraform.rs` (end-to-end):
  - Run a small Terraform plan (S3 + OAC + Distribution + object upload).
  - Run the data plane against the resulting distribution ID.
  - Assert content matches.

### 14.3 Client Compatibility

- `curl` path-based and host-based (via `--resolve`).
- `reqwest` both modes.
- `aws-sdk-s3` hitting the CloudFront data plane URL (treating it as an HTTPS CDN for S3 static content) -- the interesting edge case where tests use the CloudFront URL to verify content that would otherwise be private.
- A tiny Playwright smoke test (browser hits `http://d{id}.cloudfront.net/index.html`) -- optional, gated on CI having browser infra.

### 14.4 Non-Functional

- **Startup latency** -- data plane adds < 10 ms to cold start.
- **Per-request overhead** -- data plane handler (before origin dispatch) adds < 1 ms p50 to request latency vs direct S3 fetch. Measured informally during integration tests.

---

## 15. Phased Implementation Plan

### Phase D0 — S3 Origin Pass-Through

**Depends on:** CloudFront management Phase 0 (Distribution + OAC + Tags).

- Create `rustack-cloudfront-dataplane` crate skeleton.
- axum router with path-based routing only (`/_aws/cloudfront/{id}/*`).
- Cache-behavior selection with glob matcher.
- S3 origin dispatch in-process.
- `DefaultRootObject`, `OriginPath`, origin custom headers.
- Disabled-distribution 403, method-not-allowed 405, missing-distribution 404.
- Divergence warnings (no `FAIL_ON_FUNCTION` yet).
- `CloudFrontDataPlaneRouter` wired into the gateway.
- Integration tests: bucket + object + distribution + GET.

**Unlocks:** `curl` a distribution, E2E Terraform testing of the static-site pattern.

### Phase D1 — Custom HTTP Origin + Policies + Host Routing

**Depends on:** CloudFront management Phase 1 (Policies + OAI).

- Custom HTTP origin via `reqwest`.
- `ResponseHeadersPolicy` application (managed + custom).
- `CustomErrorResponses` handling.
- Host-based routing (`{id}.cloudfront.net` and `Aliases`).
- `FAIL_ON_FUNCTION` env var with per-feature behavior.
- OriginRequestPolicy header/cookie/query-string forwarding subset.

**Unlocks:** distributions with non-S3 origins, CORS/security-header tests, custom error pages, realistic host-header tests.

### Phase D2 — APIGW v2, Lambda URL, Compression, OriginGroup Failover

**Depends on:** management Phase 1 + APIGW v2 + Lambda execution engines.

- In-process dispatch to APIGW v2 execution engine.
- In-process dispatch to Lambda function URLs.
- Basic Brotli / gzip compression for custom origins that do not already encode.
- OriginGroup failover (primary → secondary on 5xx or connection failure).

**Unlocks:** full CloudFront → APIGW → Lambda chain tested locally; frontend API gateway patterns.

### Future (not scheduled)

- H/2 upstream to custom origins (if the cost/benefit ever changes).
- Actual ETag-based 304 Not Modified handling (the one piece of "caching" that is cheap to implement).
- Optional cache with in-memory LRU and honest TTL evaluation (contingent on users asking, with a clear performance/correctness trade-off).

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Cache-behavior glob matching differs subtly from AWS (edge cases: trailing slashes, empty path, `*` at both ends) | Medium | Medium | Comprehensive unit tests mirroring AWS docs examples; proptest against a reference implementation |
| Host-header resolution is brittle when users forget `/etc/hosts` entries | High | Low | Document path-based URL prominently; emit a helpful error when a request hits rustack with an unknown host suggesting the path-based alternative |
| In-process S3 dispatch bypasses the HTTP layer's auth checks, diverging from real AWS (where the OAC signs the S3 request) | Known | Low | Document intentionally. Tests that want to verify OAC wiring should assert on the bucket policy via S3's management API, not on the data plane. |
| Silent divergence on Lambda@Edge/Functions causes tests to pass that would fail in prod | Medium | High | Loud warnings by default; FAIL_ON_FUNCTION makes detection opt-in strict; document prominently in README |
| Custom HTTP origin reqwest client leaks TCP connections on test teardown | Low | Low | Shared client per-data-plane; `Drop` on DataPlane explicitly drops it |
| 64 MiB body cap surprises a user with a large asset | Medium | Low | Clear 413 response; env var to raise the cap |
| Host parsing confuses `foo.cloudfront.net.evil.com` as a distribution host | Low | Medium | Strict suffix match: must end in `.{suffix}` *and* have exactly one leading subdomain segment matching the distribution ID format |
| axum version bump breaks tower integration | Low | Low | Pin axum version in workspace deps; integration tests catch regressions |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users request caching ("my test relies on CloudFront caching a response for 5 seconds") | Medium | Medium | Reiterate non-goal; suggest they use `Cache-Control` on the origin + client-side caching in their test |
| Users request Lambda@Edge execution | Medium | High | Hard non-goal; direct them to running the function as a regular Lambda and asserting on its logs instead |
| Users request real TLS termination for HTTPS testing | Low | Low | Non-goal; rustack is HTTP-only. Users needing HTTPS can terminate TLS in front of rustack with a local reverse proxy (nginx, caddy) |
| Feature creep: compression, signed URLs, realtime logs | Medium | Medium | Enumerate non-goals in §3.2, reference when new requests arrive |

### 16.3 Operational Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Data plane feature adds axum transitive closure to the binary | Known | Low | `cloudfront-dataplane-s3` is a feature flag; disable for management-only builds. In practice axum is small (~200 KB in release binary) |
| Integration tests slow down due to origin dispatch latency | Low | Low | S3 in-process dispatch is sub-millisecond. Custom HTTP origin uses localhost test servers, also fast. |

---

## 17. Open Questions

- **Should we expose a simple `/_aws/cloudfront/health` endpoint that lists active distributions and their origins?** Useful for debugging. Low cost. Tentatively yes in Phase D1.
- **Should host-based routing support a `cloudfront.rustack.local` default suffix as well as `.cloudfront.net`?** Could help avoid confusion when `.cloudfront.net` entries in `/etc/hosts` get forgotten about. Tentatively yes, configurable and additive.
- **How do we surface divergence warnings when Rustack runs in Docker and users do not see `tracing` output?** Options: a dedicated `/_aws/cloudfront/divergence` endpoint that returns accumulated warnings as JSON; a header on every response (`X-Amz-Cf-Divergence: lambda-edge-skipped`). Decide during D1.
- **Should the data plane support ranged GET (`Range` header) for S3 origins?** S3 itself supports it; passing through is cheap. Likely yes in D0, tracked as a tiny follow-up if not.
- **Should failed origin requests emit to `RealtimeLogConfig`?** No for initial phases; reconsider if users ask.
