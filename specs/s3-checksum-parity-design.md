# S3 Checksum Parity: Closing the Gap with AWS S3 Behavior

**Date:** 2026-03-09
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md)
**Scope:** Bring ruststack's S3 checksum handling to full parity with AWS S3 (post-Jan 2025 default integrity protections) and vendor LocalStack reference implementation.
**Triggered by:** [GitHub Issue #3](https://github.com/tyrchen/ruststack/issues/3) -- Go AWS SDK v2 warns "Response has no supported checksum" on GetObject.

---

## Table of Contents

1. [Background and Root Cause Analysis](#1-background-and-root-cause-analysis)
2. [AWS S3 Checksum Behavior (2025)](#2-aws-s3-checksum-behavior-2025)
3. [Gap Analysis: Ruststack vs LocalStack vs AWS](#3-gap-analysis-ruststack-vs-localstack-vs-aws)
4. [Goals and Non-Goals](#4-goals-and-non-goals)
5. [Design: Phase 1 -- Core Checksum Fixes](#5-design-phase-1----core-checksum-fixes)
6. [Design: Phase 2 -- CRC64NVME and Checksum Types](#6-design-phase-2----crc64nvme-and-checksum-types)
7. [Design: Phase 3 -- AWS-Chunked Trailing Headers](#7-design-phase-3----aws-chunked-trailing-headers)
8. [Design: Phase 4 -- Multipart Checksum Parity](#8-design-phase-4----multipart-checksum-parity)
9. [Data Model Changes](#9-data-model-changes)
10. [Test Plan](#10-test-plan)
11. [Migration and Backward Compatibility](#11-migration-and-backward-compatibility)
12. [Implementation Order](#12-implementation-order)

---

## 1. Background and Root Cause Analysis

### 1.1 The Reported Bug

A user started ruststack with S3 service and used the Go AWS SDK v2 to PutObject then GetObject. The GetObject succeeded but the SDK logged:

```
SDK WARN Response has no supported checksum. Not validating response payload.
```

The object data was returned correctly, but the SDK could not validate its integrity because ruststack returned no `x-amz-checksum-*` headers.

### 1.2 Root Cause: Wiring Gap

The checksum infrastructure was **fully built** across three layers but never connected end-to-end:

| Layer | Status | Location |
|-------|--------|----------|
| Checksum computation (CRC32, CRC32C, SHA1, SHA256) | Working | `ruststack-s3-core/src/checksums.rs` |
| `ChecksumData` storage in `S3Object` model | Working | `ruststack-s3-core/src/state/object.rs:185-193` |
| HTTP response serialization (`x-amz-checksum-*` headers) | Working | `ruststack-s3-http/src/response.rs:287-292` |
| GetObjectOutput / HeadObjectOutput checksum fields | Defined | `ruststack-s3-model/src/output/object.rs:70-81` |
| **PutObject handler: compute checksum when client doesn't provide one** | **Missing** | `ruststack-s3-core/src/ops/object.rs:142` |
| **GetObject handler: populate checksum fields from stored data** | **Missing** | `ruststack-s3-core/src/ops/object.rs:312-346` |
| **HeadObject handler: populate checksum fields from stored data** | **Missing** | `ruststack-s3-core/src/ops/object.rs:413-449` |

The `handle_get_object()` destructured 8 fields from the `S3Object` but skipped `checksum`. The `handle_put_object()` only stored client-provided checksums and never auto-computed a default. The HTTP serialization layer would have returned the headers correctly if the output fields had been populated.

### 1.3 Why No Test Caught It

**Three compounding factors:**

1. **Zero checksum assertions in the test suite.** All 25+ S3 integration tests in `tests/integration/src/test_object.rs` only assert on `content_type`, `content_length`, `e_tag`, `last_modified`, and body content. No test inspects any `checksum_*` field or `x-amz-checksum-*` header.

2. **The Rust AWS SDK silently tolerates missing checksums.** Unlike the Go SDK v2 which logs a warning, the Rust SDK with `BehaviorVersion::latest()` does not surface missing checksums. The `get_object().send().await` call succeeds silently, making the gap invisible.

3. **No third-party conformance suite was integrated.** The `docs/research/s3-integration-test-suites-research.md` surveyed suites (Ceph s3-tests, MinIO Mint, s3s-e2e) but none were adopted. These suites test checksum behavior comprehensively.

### 1.4 Initial Fix (PR #4)

PR #4 addressed the immediate issue:
- Auto-compute CRC32 in PutObject when no client checksum is provided
- Populate checksum fields in GetObject, HeadObject, and PutObject responses
- Preserve source checksums in CopyObject

This fix resolves the reported issue but exposes deeper gaps that this spec addresses.

---

## 2. AWS S3 Checksum Behavior (2025)

### 2.1 Default Integrity Protections (Effective ~March 2025)

AWS announced "default data integrity protections for new objects in Amazon S3" in December 2024. Key changes:

- **All new uploads get a checksum.** If the SDK sends a CRC-based checksum, S3 validates it server-side. If no checksum is provided, S3 computes **CRC64NVME** server-side and stores it.
- **SDKs send checksums by default.** Modern SDKs (Go v2 >= v1.73.0, Rust >= v1.69.0) automatically compute and send CRC32 on uploads.
- **SDKs request checksums by default.** Modern SDKs send `x-amz-checksum-mode: ENABLED` on GetObject/HeadObject and validate the returned checksum.

### 2.2 Supported Algorithms

| Algorithm | Bit Width | Multipart Types | SDK Default | Server Default | Notes |
|-----------|-----------|-----------------|-------------|----------------|-------|
| CRC64NVME | 64 | FULL_OBJECT only | CLI only | Yes | NVMe polynomial; fastest on modern hardware |
| CRC32 | 32 | COMPOSITE, FULL_OBJECT | Most SDKs | No | IEEE 802.3; broadest compatibility |
| CRC32C | 32 | COMPOSITE, FULL_OBJECT | No | No | Castagnoli; hardware-accelerated on x86 |
| SHA-1 | 160 | COMPOSITE only | No | No | Not linearizable for FULL_OBJECT |
| SHA-256 | 256 | COMPOSITE only | No | No | Not linearizable for FULL_OBJECT |

### 2.3 Checksum Type: COMPOSITE vs FULL_OBJECT

For multipart uploads, S3 supports two checksum modes:

| Property | FULL_OBJECT | COMPOSITE |
|----------|------------|-----------|
| Definition | Checksum of entire object content across all parts | Checksum-of-checksums (computed from individual part checksums) |
| Computation | CRC combination via Galois Field (GF(2)) math | Concatenate raw part checksums, hash the result, append `-N` |
| Supported by | CRC64NVME (forced), CRC32, CRC32C | CRC32, CRC32C, SHA-1, SHA-256 |
| Constraints | CRC64NVME → FULL_OBJECT only; SHA-1/SHA-256 → not supported | CRC64NVME → not supported |

### 2.4 `ChecksumMode` Header

GetObject and HeadObject only return checksum headers when the client sends `x-amz-checksum-mode: ENABLED`. This is important for backward compatibility with older SDKs that don't expect checksum headers.

### 2.5 AWS-Chunked Transfer Encoding

Modern SDKs use aws-chunked encoding for streaming uploads with trailing checksum headers:

```
<hex-chunk-size>;chunk-signature=<sig>\r\n
<chunk-data>\r\n
...
0;chunk-signature=<sig>\r\n
x-amz-checksum-crc32:<base64-value>\r\n
x-amz-trailer-signature:<sig>\r\n
\r\n
```

The `x-amz-trailer` request header declares which trailing header will follow. The checksum value is transmitted AFTER all body data, allowing the SDK to compute it incrementally during streaming.

---

## 3. Gap Analysis: Ruststack vs LocalStack vs AWS

### 3.1 Feature Matrix

| Feature | AWS S3 | LocalStack (vendor) | Ruststack (current) | Gap Severity |
|---------|--------|-------------------|---------------------|--------------|
| Auto-compute checksum on PutObject | CRC64NVME | Yes (configurable) | CRC32 (PR #4) | Medium |
| Return checksums in GetObject/HeadObject | Yes | Yes | Yes (PR #4) | Fixed |
| `ChecksumMode=ENABLED` gating | Yes | Yes (`provider.py:1042`) | No (always returns) | Medium |
| CRC64NVME algorithm | Yes | Yes | **No** | **High** |
| AWS-chunked trailing header extraction | Yes | Yes (`codec.py`) | **No** (headers lost) | **Critical** |
| COMPOSITE vs FULL_OBJECT types | Yes | Yes (`models.py:518-623`) | **No** | **High** |
| Multipart CRC combination (GF(2) math) | Yes | Yes (`checksums.py:34-166`) | **No** | **High** |
| Part-level checksum storage | Yes | Yes | **No** | **High** |
| Multipart complete checksum validation | Yes | Yes | **No** | Medium |
| GetObjectAttributes with checksums | Yes | Yes (`provider.py:2148-2247`) | **No** (returns None) | Low |
| Checksum validation on PutObject | Yes | Yes | **No** | Medium |
| PutObject response checksum headers | Yes | Yes | Yes (PR #4) | Fixed |
| CopyObject checksum preservation | Yes | Yes | Yes (PR #4) | Fixed |

### 3.2 Critical Bugs Found During Analysis

#### Bug 1: AWS-Chunked Trailing Headers Are Silently Dropped

**Location:** `ruststack-s3-http/src/codec.rs:45-118`

The `decode_aws_chunked()` function strips the aws-chunked framing and returns decoded body bytes, but **silently discards all trailing headers**. This means:

- When the Go SDK v2 or Rust SDK sends a PutObject with `Content-Encoding: aws-chunked` and a trailing `x-amz-checksum-crc32` header, the checksum value is **lost**.
- The object is stored without any checksum (before PR #4) or with a server-computed CRC32 (after PR #4) that may differ from what the client computed.
- No checksum validation occurs, so data corruption during transit would go undetected.

This is the most critical bug because it breaks the fundamental data integrity guarantee that checksums provide.

#### Bug 2: Multipart Complete Ignores Checksums Entirely

**Location:** `ruststack-s3-core/src/ops/multipart.rs:403`

The `handle_complete_multipart_upload()` handler hardcodes `checksum: None` in the assembled `S3Object`, discarding any per-part checksums. No checksum combination (composite or full-object) is performed.

#### Bug 3: GetObjectAttributes Returns No Checksum Data

**Location:** `ruststack-s3-core/src/ops/object_config.rs:586`

The `handle_get_object_attributes()` handler hardcodes `checksum: None`, even when the stored object has checksum data.

### 3.3 Behavioral Differences

| Behavior | AWS S3 / LocalStack | Ruststack |
|----------|-------------------|-----------|
| Default algorithm when client sends none | CRC64NVME (AWS), CRC32 (Ruststack PR #4) | Acceptable divergence (CRC32 is fine for local testing) |
| Checksums in GetObject response | Only when `ChecksumMode=ENABLED` | Always returned (minor divergence) |
| Checksum validation on upload | Validates client checksum against server-computed | No validation |
| Multiple checksums in single request | Rejects (InvalidRequest) | Accepts first found |

---

## 4. Goals and Non-Goals

### Goals

1. **Full checksum lifecycle parity** -- objects uploaded through any path (PutObject, CopyObject, multipart) store and return checksums correctly.
2. **CRC64NVME support** -- implement the algorithm AWS now uses as the server-side default.
3. **AWS-chunked trailing header extraction** -- stop discarding trailing headers; extract and validate checksums from them.
4. **Multipart checksum parity** -- implement COMPOSITE and FULL_OBJECT checksum types, including GF(2) CRC combination math.
5. **ChecksumMode gating** -- only return checksums in GetObject/HeadObject when `ChecksumMode=ENABLED`, matching AWS behavior.
6. **Checksum validation on upload** -- validate client-provided checksums against server-computed values.
7. **Comprehensive test coverage** -- add tests for every checksum path, including aws-chunked, multipart, and edge cases.

### Non-Goals

1. **Full S3 conformance test suite integration** -- desirable but out of scope for this spec. File a follow-up.
2. **Checksum support for SelectObjectContent** -- not implemented in ruststack at all.
3. **Trailer signature validation** -- we currently skip SigV4 validation entirely; trailer signatures are included in this skip.
4. **Retroactive checksum computation** -- existing objects stored before this change will not get checksums. This matches AWS behavior (only new objects get default checksums).

---

## 5. Design: Phase 1 -- Core Checksum Fixes

Phase 1 addresses behavioral correctness for single-object operations.

### 5.1 ChecksumMode Gating in GetObject/HeadObject

**Current behavior (PR #4):** Always return checksum headers.
**Target behavior:** Only return checksum headers when `input.checksum_mode == Some(ChecksumMode::Enabled)`.

```rust
// In handle_get_object():
let cksum = if input.checksum_mode.as_ref().is_some_and(|m| m.as_str() == "ENABLED") {
    obj_checksum.as_ref().map(checksum_to_fields)
} else {
    None
};
```

**Rationale:** Modern SDKs always send `ChecksumMode=ENABLED`, so this works correctly with Go/Rust/Python/JS SDKs. Older SDKs that don't send this header won't get unexpected checksum headers. This matches AWS behavior and the LocalStack reference (`provider.py:1042`).

Apply the same logic to `handle_head_object()`.

### 5.2 Checksum Validation on PutObject

When the client provides a checksum value, validate it against the server-computed value.

```rust
// In handle_put_object(), after write:
if let Some(client_checksum) = extract_checksum_from_put(&input) {
    let algorithm = ChecksumAlgorithm::from_str(&client_checksum.algorithm)
        .map_err(|_| S3ServiceError::InvalidRequest("unsupported checksum algorithm".into()))?;
    let computed = compute_checksum(algorithm, &body_data);
    if client_checksum.value != computed {
        return Err(S3ServiceError::BadDigest.into_s3_error());
    }
}
```

**Error type:** `BadDigest` (HTTP 400), matching AWS behavior.

### 5.3 Reject Multiple Checksum Headers

When a client sends more than one `x-amz-checksum-*` header (e.g., both CRC32 and SHA256), reject with `InvalidRequest`. Currently `extract_checksum_from_put()` silently takes the first one found.

```rust
fn extract_checksum_from_put(input: &PutObjectInput) -> Result<Option<ChecksumData>, S3ServiceError> {
    let candidates = [
        ("CRC32", &input.checksum_crc32),
        ("CRC32C", &input.checksum_crc32c),
        ("CRC64NVME", &input.checksum_crc64nvme),  // Phase 2
        ("SHA1", &input.checksum_sha1),
        ("SHA256", &input.checksum_sha256),
    ];
    let found: Vec<_> = candidates.iter().filter(|(_, v)| v.is_some()).collect();
    if found.len() > 1 {
        return Err(S3ServiceError::InvalidRequest(
            "Only one checksum value can be provided per request".into(),
        ));
    }
    Ok(found.into_iter().next().map(|(alg, val)| ChecksumData {
        algorithm: (*alg).to_owned(),
        value: val.as_ref().unwrap().clone(),
    }))
}
```

### 5.4 GetObjectAttributes Checksum Support

Populate the `checksum` field in `GetObjectAttributesOutput` from the stored `S3Object.checksum`:

```rust
// In handle_get_object_attributes():
let checksum = if requested_attrs.contains(&ObjectAttributesPart::Checksum) {
    obj.checksum.as_ref().map(|c| ObjectChecksum {
        algorithm: Some(c.algorithm.clone()),
        value: Some(c.value.clone()),
        checksum_type: Some(ChecksumType::FullObject),
    })
} else {
    None
};
```

---

## 6. Design: Phase 2 -- CRC64NVME and Checksum Types

### 6.1 CRC64NVME Algorithm

Add CRC64NVME to `ChecksumAlgorithm` and `compute_checksum()`.

**Crate:** Use `crc64fast-nvme` (pure Rust, hardware-accelerated on x86_64/aarch64).

```toml
# Cargo.toml
crc64fast-nvme = "1"
```

```rust
// In checksums.rs:
pub enum ChecksumAlgorithm {
    Crc32,
    Crc32c,
    Crc64Nvme,  // New
    Sha1,
    Sha256,
}

pub fn compute_checksum(algorithm: ChecksumAlgorithm, data: &[u8]) -> String {
    match algorithm {
        // ... existing arms ...
        ChecksumAlgorithm::Crc64Nvme => {
            let value = crc64fast_nvme::digest(data);
            BASE64_STANDARD.encode(value.to_be_bytes())
        }
    }
}
```

**Header mapping:**
- Request header: `x-amz-checksum-crc64nvme`
- Response header: `x-amz-checksum-crc64nvme`
- Model field: `checksum_crc64nvme: Option<String>`

### 6.2 ChecksumType in S3Object Model

Add `checksum_type` field to `ChecksumData` and `S3Object`:

```rust
/// Checksum data attached to an S3 object or part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumData {
    pub algorithm: String,
    pub value: String,
    /// Whether this is a FULL_OBJECT or COMPOSITE checksum.
    #[serde(default = "default_checksum_type")]
    pub checksum_type: String,
}

fn default_checksum_type() -> String {
    "FULL_OBJECT".to_owned()
}
```

### 6.3 Default Algorithm Selection

For PutObject when no client checksum is provided:
- Use **CRC32** as the default (matching what most SDKs send).
- AWS uses CRC64NVME server-side, but CRC32 is pragmatic for a local testing tool since all SDKs support it natively.
- Make this configurable via server config if needed in the future.

### 6.4 Update All Checksum Extraction Points

Update `extract_checksum_from_put()`, HTTP request parsing, and response serialization to handle CRC64NVME:

- `ruststack-s3-http/src/request.rs`: Parse `x-amz-checksum-crc64nvme` header
- `ruststack-s3-http/src/response.rs`: Serialize `checksum_crc64nvme` field to header
- `ruststack-s3-model/src/input/object.rs`: Add `checksum_crc64nvme` to `PutObjectInput`
- `ruststack-s3-model/src/output/object.rs`: Already has `checksum_crc64nvme` in `GetObjectOutput`

---

## 7. Design: Phase 3 -- AWS-Chunked Trailing Headers

### 7.1 Problem

The current `decode_aws_chunked()` in `ruststack-s3-http/src/codec.rs` strips chunked framing and returns decoded body bytes, but **discards all trailing headers**. When SDKs send checksums via trailing headers (the default for streaming uploads), those checksums are lost.

### 7.2 Solution: Return Trailing Headers

Change `decode_aws_chunked()` to return both body data and trailing headers:

```rust
/// Result of decoding an AWS-chunked body.
#[derive(Debug)]
pub struct AwsChunkedResult {
    /// The decoded body data.
    pub body: Bytes,
    /// Trailing headers extracted from the chunked stream.
    pub trailing_headers: HashMap<String, String>,
}

/// Decode an AWS-chunked encoded body, extracting trailing headers.
pub fn decode_aws_chunked(data: &[u8]) -> Result<AwsChunkedResult, AwsChunkedError> {
    let mut decoded = Vec::new();
    let mut pos = 0;

    loop {
        // Parse chunk size line: <hex>;chunk-signature=<sig>\r\n
        let line_end = find_crlf(data, pos)?;
        let size_str = extract_chunk_size(&data[pos..line_end]);
        let chunk_size = usize::from_str_radix(size_str, 16)?;
        pos = line_end + 2; // skip \r\n

        if chunk_size == 0 {
            break;
        }

        decoded.extend_from_slice(&data[pos..pos + chunk_size]);
        pos += chunk_size + 2; // skip data + \r\n
    }

    // Parse trailing headers (everything after the terminal chunk)
    let trailing_headers = parse_trailing_headers(&data[pos..])?;

    Ok(AwsChunkedResult {
        body: Bytes::from(decoded),
        trailing_headers,
    })
}

/// Parse trailing headers from the remainder of an AWS-chunked stream.
///
/// Format: `header-name:value\r\n` repeated, terminated by `\r\n`.
fn parse_trailing_headers(data: &[u8]) -> Result<HashMap<String, String>, AwsChunkedError> {
    let mut headers = HashMap::new();
    let text = std::str::from_utf8(data)?;

    for line in text.split("\r\n") {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip trailer signature lines
        if line.starts_with("x-amz-trailer-signature") {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(
                key.trim().to_lowercase(),
                value.trim().to_owned(),
            );
        }
    }

    Ok(headers)
}
```

### 7.3 Integrate Trailing Headers in PutObject

Update the PutObject request parsing to extract checksums from trailing headers:

```rust
// In the HTTP layer, after decoding aws-chunked:
if is_aws_chunked(&headers) {
    let result = decode_aws_chunked(&body_data)?;
    body_data = result.body;

    // Extract checksum from trailing headers if not already in request headers
    if input.checksum_crc32.is_none() {
        if let Some(v) = result.trailing_headers.get("x-amz-checksum-crc32") {
            input.checksum_crc32 = Some(v.clone());
        }
    }
    // ... same for crc32c, crc64nvme, sha1, sha256
}
```

### 7.4 Trailing Header Declaration

The `x-amz-trailer` request header declares which trailing header will follow. Use this to know which checksum to extract:

```rust
// Parse the x-amz-trailer header to know which checksum algorithm to expect
let trailer_header = headers.get("x-amz-trailer").map(|v| v.to_str().unwrap_or(""));
```

---

## 8. Design: Phase 4 -- Multipart Checksum Parity

### 8.1 Part-Level Checksum Storage

Add checksum data to `S3Part`:

```rust
// In state/multipart.rs or state/object.rs:
pub struct S3Part {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
    /// Optional checksum for this part.
    pub checksum: Option<ChecksumData>,
}
```

### 8.2 CreateMultipartUpload: Store Checksum Config

Store `checksum_algorithm` and `checksum_type` on the `S3Multipart` record:

```rust
pub struct S3Multipart {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub metadata: ObjectMetadata,
    pub owner: Owner,
    pub storage_class: String,
    /// Checksum algorithm declared at multipart creation.
    pub checksum_algorithm: Option<String>,
    /// Checksum type (COMPOSITE or FULL_OBJECT).
    pub checksum_type: Option<String>,
    pub parts: BTreeMap<u32, S3Part>,
}
```

Validation rules at creation time:
- CRC64NVME → force `checksum_type = FULL_OBJECT`
- SHA-1, SHA-256 → force `checksum_type = COMPOSITE`
- CRC32, CRC32C → default to COMPOSITE, allow FULL_OBJECT if specified

### 8.3 UploadPart: Store Part Checksums

When uploading a part:
1. Extract checksum from request headers (or trailing headers for aws-chunked)
2. Validate algorithm matches the multipart upload's `checksum_algorithm`
3. If client provides checksum, validate against server-computed value
4. Store checksum in `S3Part.checksum`

### 8.4 CompleteMultipartUpload: Checksum Combination

Implement the two combination strategies:

#### COMPOSITE (default for CRC32, CRC32C, SHA-1, SHA-256)

```rust
fn compute_composite_checksum(
    algorithm: ChecksumAlgorithm,
    parts: &[S3Part],
) -> String {
    // Concatenate raw (decoded) checksums of each part
    let mut combined = Vec::new();
    for part in parts {
        if let Some(ref cksum) = part.checksum {
            if let Ok(bytes) = BASE64_STANDARD.decode(&cksum.value) {
                combined.extend_from_slice(&bytes);
            }
        }
    }
    // Hash the concatenation
    let result = compute_checksum(algorithm, &combined);
    format!("{result}-{}", parts.len())
}
```

This already exists in `checksums.rs` as `compute_composite_checksum()`.

#### FULL_OBJECT (required for CRC64NVME, optional for CRC32/CRC32C)

CRC checksums are "linearizable" -- two CRC values can be combined mathematically using Galois Field (GF(2)) matrix operations, given the length of the second data block. This avoids re-reading all part data.

```rust
/// Combine two CRC values using GF(2) matrix multiplication.
///
/// Given `crc1 = CRC(data1)` and `crc2 = CRC(data2)` and `len2 = data2.len()`,
/// computes `CRC(data1 || data2)` without access to the original data.
pub fn combine_crc32(crc1: u32, crc2: u32, len2: u64) -> u32 {
    // GF(2) matrix squaring to compute x^(8*len2) mod polynomial
    // Then matrix-vector multiply to shift crc1 by len2 bytes
    // Finally XOR with crc2
    // Implementation follows the zlib crc32_combine algorithm.
    gf2_combine(crc1, crc2, len2, CRC32_POLYNOMIAL)
}

pub fn combine_crc32c(crc1: u32, crc2: u32, len2: u64) -> u32 {
    gf2_combine(crc1, crc2, len2, CRC32C_POLYNOMIAL)
}

pub fn combine_crc64_nvme(crc1: u64, crc2: u64, len2: u64) -> u64 {
    gf2_combine_64(crc1, crc2, len2, CRC64NVME_POLYNOMIAL)
}
```

**Reference implementation:** `vendors/localstack/localstack-core/localstack/services/s3/checksums.py:34-166`

### 8.5 CompleteMultipartUpload Handler Changes

```rust
// In handle_complete_multipart_upload():
let checksum = if let Some(ref algorithm) = multipart.checksum_algorithm {
    let algo = ChecksumAlgorithm::from_str(algorithm)?;
    let checksum_type = multipart.checksum_type.as_deref().unwrap_or("COMPOSITE");

    let value = match checksum_type {
        "FULL_OBJECT" => combine_part_checksums_full_object(algo, &ordered_parts)?,
        _ => compute_composite_checksum(algo, &part_checksums),
    };

    Some(ChecksumData {
        algorithm: algorithm.clone(),
        value,
        checksum_type: checksum_type.to_owned(),
    })
} else {
    None
};
```

### 8.6 Validation on CompleteMultipartUpload

When `checksum_algorithm` is set on the multipart upload:
- Each part MUST have a matching checksum (reject with `InvalidRequest` if missing)
- Part checksum algorithm MUST match the multipart's algorithm
- If the complete request includes a `validation_checksum`, compare against the computed value

---

## 9. Data Model Changes

### 9.1 ChecksumData (state/object.rs)

```rust
// Before:
pub struct ChecksumData {
    pub algorithm: String,
    pub value: String,
}

// After:
pub struct ChecksumData {
    pub algorithm: String,
    pub value: String,
    #[serde(default = "default_full_object")]
    pub checksum_type: String,  // "FULL_OBJECT" or "COMPOSITE"
}
```

### 9.2 S3Part (new or extended)

Add `checksum: Option<ChecksumData>` field.

### 9.3 S3Multipart (state/multipart.rs)

Add:
- `checksum_algorithm: Option<String>`
- `checksum_type: Option<String>`

### 9.4 ChecksumAlgorithm Enum (checksums.rs)

Add `Crc64Nvme` variant. Update `from_str()`, `as_str()`, `compute_checksum()`.

### 9.5 New Crate Dependency

```toml
crc64fast-nvme = "1"
```

---

## 10. Test Plan

### 10.1 Unit Tests (in respective modules)

**Checksum computation (`checksums.rs`):**
- `test_should_compute_crc64nvme_checksum`
- `test_should_combine_crc32_checksums`
- `test_should_combine_crc32c_checksums`
- `test_should_combine_crc64nvme_checksums`
- `test_should_compute_composite_checksum_with_suffix`

**AWS-chunked codec (`codec.rs`):**
- `test_should_decode_aws_chunked_with_trailing_headers`
- `test_should_extract_checksum_from_trailing_headers`
- `test_should_handle_empty_trailing_headers`
- `test_should_handle_trailer_signature`

**Checksum extraction (`ops/object.rs`):**
- `test_should_reject_multiple_checksum_headers`
- `test_should_extract_crc64nvme_checksum`

### 10.2 Integration Tests (tests/integration)

**Single-object checksum lifecycle:**
- `test_should_put_and_get_object_with_default_checksum` -- PutObject without explicit checksum, GetObject with `ChecksumMode::Enabled`, verify CRC32 returned
- `test_should_put_and_get_object_with_crc32c_checksum` -- PutObject with explicit CRC32C, GetObject verifies same value
- `test_should_put_and_get_object_with_sha256_checksum` -- Same for SHA-256
- `test_should_put_and_get_object_with_crc64nvme_checksum` -- Same for CRC64NVME
- `test_should_not_return_checksum_without_checksum_mode` -- GetObject without `ChecksumMode`, verify no checksum headers
- `test_should_validate_checksum_on_put` -- PutObject with wrong checksum value, expect `BadDigest`

**HeadObject:**
- `test_should_head_object_with_checksum_mode_enabled` -- Verify checksum returned
- `test_should_head_object_without_checksum_mode` -- Verify no checksum

**CopyObject:**
- `test_should_copy_object_preserving_checksum` -- Copy and verify checksum preserved

**Multipart upload:**
- `test_should_complete_multipart_with_composite_crc32_checksum` -- Upload parts with CRC32 checksums, complete, verify composite checksum
- `test_should_complete_multipart_with_full_object_crc32_checksum` -- Same with FULL_OBJECT type
- `test_should_reject_part_without_checksum_when_required` -- Upload part without checksum when multipart has `checksum_algorithm`
- `test_should_reject_mismatched_part_checksum_algorithm`

**GetObjectAttributes:**
- `test_should_get_object_attributes_with_checksum`

---

## 11. Migration and Backward Compatibility

### 11.1 Stored Data

Objects stored before these changes have `checksum: None` (or `checksum: Some(...)` if stored after PR #4). No migration is needed -- GetObject will simply not return checksum headers for old objects when `ChecksumMode=ENABLED` (since there's no checksum stored).

### 11.2 Serde Compatibility

The `ChecksumData` struct gains a new `checksum_type` field with `#[serde(default)]`. Existing serialized data (if any persistence is used) will deserialize with `checksum_type = "FULL_OBJECT"` by default. This is correct for single-object uploads.

### 11.3 API Behavior Change

**Breaking change:** After Phase 1, GetObject/HeadObject will stop returning checksum headers by default (only when `ChecksumMode=ENABLED`). Modern SDKs send this header automatically, so this is transparent. Direct HTTP clients that relied on PR #4's always-return behavior will need to add the header.

---

## 12. Implementation Order

```
Phase 1 (Core fixes)                    ~2-3 days
├── 1a. ChecksumMode gating in GetObject/HeadObject
├── 1b. Checksum validation on PutObject (BadDigest)
├── 1c. Reject multiple checksum headers
├── 1d. GetObjectAttributes checksum support
└── 1e. Integration tests for all of Phase 1

Phase 2 (CRC64NVME + checksum types)    ~2-3 days
├── 2a. Add crc64fast-nvme dependency
├── 2b. CRC64NVME in ChecksumAlgorithm enum
├── 2c. ChecksumType field in ChecksumData model
├── 2d. Update HTTP request/response for CRC64NVME
├── 2e. Update extract_checksum_from_put for CRC64NVME
└── 2f. Unit + integration tests

Phase 3 (AWS-chunked trailing headers)  ~3-4 days
├── 3a. Refactor decode_aws_chunked to return trailing headers
├── 3b. Integrate trailing header extraction in PutObject path
├── 3c. Integrate in UploadPart path
├── 3d. Parse x-amz-trailer header
└── 3e. Unit + integration tests

Phase 4 (Multipart checksum parity)     ~4-5 days
├── 4a. Add checksum fields to S3Part and S3Multipart
├── 4b. CreateMultipartUpload: store checksum config
├── 4c. UploadPart: store and validate part checksums
├── 4d. Implement GF(2) CRC combination functions
├── 4e. CompleteMultipartUpload: composite checksum assembly
├── 4f. CompleteMultipartUpload: full-object CRC combination
├── 4g. Part checksum validation
└── 4h. Integration tests for all multipart paths
```

Each phase is independently shippable and testable. Phase 1 should be done immediately as it corrects behavioral issues introduced by PR #4.
