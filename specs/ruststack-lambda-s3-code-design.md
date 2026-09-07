# Rustack Lambda S3 Code Packages — Design

| | |
|---|---|
| **Status** | Draft |
| **Type** | Design |
| **Issue** | #34 — CreateFunction accepts S3Bucket/S3Key code but never downloads it |
| **Scope** | rustack-lambda-core, apps/rustack wiring, integration tests |

## 1. Problem Statement

CreateFunction (and UpdateFunctionCode) accept a Code.S3Bucket / Code.S3Key
package, return 200, and create a function record — but the S3 object is
never downloaded or stored. The function ends up with code_path = None, and
the first Invoke fails with InvalidCode("missing code root").

This makes the function silently unusable: creation reports success and
GetFunction even returns a (fake) S3 code_location, so clients have no way to
know the code was dropped until invocation fails. The failure mode hits
standard AWS SDK clients (e.g. AWS SDK for Java v2) that create functions
from an S3 package, which is the canonical deployment flow on real AWS and
in LocalStack.

This limitation is already documented in the spec set:
specs/ruststack-lambda-design.md:126 ("S3-based deployment packages — accept
S3Bucket/S3Key parameters but do not fetch from S3") and
specs/ruststack-lambda-executor-design.md:77 ("S3 code source (still
rejected; only ZipFile and ImageUri)").

### 1.1 Goals

1. **Download S3 code packages** — when CreateFunction / UpdateFunctionCode
   receives S3Bucket/S3Key, download the object from rustack's own S3 service
   and store it through the exact same path as inline ZipFile
   (FunctionStore::store_zip_code), so code_path, code_sha256, and code_size
   are populated and Invoke works.
2. **Fail fast with AWS-compatible errors** — invalid S3 references (missing
   bucket / key / object / version) and unsupported configurations (S3
   service disabled) must be rejected at CreateFunction time with
   InvalidParameterValueException, never deferred to invoke.
3. **Keep the codebase architecture intact** — follow the established
   cross-service bridge pattern (SNS->SQS SqsPublisher, EventBridge->SQS
   TargetDelivery): trait in the consuming core crate, implementation in the
   server binary, no core->core dependency.

### 1.2 Non-Goals

- **External S3 endpoints** (LocalStack, real AWS S3, MinIO as the code
  source) — out of scope for this change. The S3CodeFetcher trait is the
  seam a future HTTP-backed fetcher can slot into without touching the
  provider. See section 8.
- **Code signing configuration** (CodeSigningConfigArn) enforcement.
- **Layer version S3 code** (PublishLayerVersion with Content.S3Bucket) —
  separate gap, tracked in specs/service-operations-gap-impl-plan.md.

## 2. Current Behavior

RustackLambda::process_code (crates/rustack-lambda-core/src/provider.rs)
branches on zip_file_b64 and image_uri only. An S3Bucket-only request falls
through to the "no code provided" branch:

```text
CreateFunction (S3 code) ──▶ validate code present (S3Bucket is Some) ✔
        │
        ▼
process_code(zip=None, image=None)
        │
        ▼
"no code provided" branch ──▶ code_path = None, sha256(""), size = 0
        │
        ▼
store record (state = Active, GetFunction reports fake S3 location)
        │
        ▼
Invoke ──▶ native executor ──▶ ExecutorError::InvalidCode("missing code root")
```

## 3. Design

### 3.1 New module: rustack-lambda-core::code

A new module code.rs in rustack-lambda-core defines the seam between the
Lambda provider and wherever code bytes come from:

```rust
//! S3 code fetching seam for Lambda deployment packages.
//!
//! Uses async-trait because RustackLambda stores the fetcher as
//! Arc<dyn S3CodeFetcher> (object-safe dynamic dispatch), which native
//! async fn in traits cannot express (per AGENTS.md section Async & Concurrency).

/// Errors while fetching function code from S3.
#[derive(Debug, thiserror::Error)]
pub enum S3CodeFetchError {
    #[error("bucket not found: {bucket}")]
    BucketNotFound { bucket: String },
    #[error("object not found: bucket={bucket}, key={key}")]
    ObjectNotFound { bucket: String, key: String },
    #[error("object version not found: bucket={bucket}, key={key}, version={version}")]
    VersionNotFound { bucket: String, key: String, version: String },
    #[error("internal error fetching code from S3: {0}")]
    Internal(#[source] anyhow::Error),
}

/// Fetches deployment package bytes from an S3 location.
#[async_trait]
pub trait S3CodeFetcher: Send + Sync {
    /// Download the object at bucket/key, optionally a specific version.
    async fn fetch_code(
        &self,
        bucket: &str,
        key: &str,
        version: Option<&str>,
    ) -> Result<Bytes, S3CodeFetchError>;
}
```

Design points:

- **Error variants mirror the S3 failure taxonomy** the provider must map to
  AWS Lambda error messages (section 4): bucket missing, object missing,
  version missing, internal. A single Internal variant carries the source
  error via #[source] (per AGENTS.md section Error Handling).
- **UnavailableS3CodeFetcher** — the default fetcher used when the app has no
  S3 provider (S3 service disabled or feature not compiled). It always
  returns S3CodeFetchError::Internal with a message telling the user to
  enable the S3 service or use ZipFile. The provider maps that to a clear
  InvalidParameterValueException (section 4.5).

### 3.2 Provider changes (rustack-lambda-core::provider)

RustackLambda gains a field:

```rust
pub struct RustackLambda {
    // ...existing fields...
    code_fetcher: Arc<dyn S3CodeFetcher>,
}
```

- Existing constructors (new, with_store, with_executor) keep their
  signatures and install Arc::new(UnavailableS3CodeFetcher) so no existing
  caller breaks.
- New builder with_code_fetcher(mut self, fetcher: Arc<dyn S3CodeFetcher>)
  -> Self — used by the app wiring (section 5) and integration tests
  (section 6).
- The snapshot types (LambdaSnapshot, store snapshots) are unaffected:
  code_fetcher is a stateless dependency, not persisted state.

### 3.3 process_code takes a CodeSource

To stay under the clippy argument-count limit and share one field mapping
between the two call sites, the code inputs are bundled into a private
CodeSource struct; process_code signature becomes:

```rust
struct CodeSource<'a> {
    zip_file_b64: Option<&'a str>,
    s3_bucket: Option<&'a str>,
    s3_key: Option<&'a str>,
    s3_object_version: Option<&'a str>,
    image_uri: Option<&'a str>,
}

async fn process_code(
    &self,
    function_name: &str,
    version: &str,
    source: CodeSource<'_>,
) -> Result<(String, u64, Option<Bytes>, Option<PathBuf>, Option<String>), LambdaServiceError>
```

New branch, ordered after zip_file_b64 and before image_uri:

```text
s3_bucket is Some
    │ 1. validate s3_key present          (else InvalidParameterValue)
    │ 2. code_fetcher.fetch_code(bucket, key, version).await   (bounded by
    │    │    a per-read timeout; error ──▶ map to InvalidParameterValue)
    │    ▼ ok
    │ 3. check_code_size(len)             (> 50 MB ──▶ InvalidParameterValue,
    │    │                                  BEFORE any storage/extraction)
    │    ▼ ok
    │ 4. store_zip_code(function_name, version, &bytes)   ← same path as ZipFile
    │       (validates zip structure, rejects path traversal,
    │        caps extracted bytes at 250 MB to defeat zip bombs,
    │        computes sha256 + size, extracts bootstrap)
    │       ▼
    └─▶ (sha256, size, Some(bytes), Some(code_path), None)
```

This guarantees S3 code and inline ZipFile code are byte-for-byte equivalent
downstream: same validation, same storage, same executor path.

### 3.4 Validation rules (create_function, update_function_code)

| Condition | Result |
|---|---|
| S3Bucket set, S3Key missing | InvalidParameterValue: "Code.S3Key is required when Code.S3Bucket is provided" |
| S3Key set, S3Bucket missing | InvalidParameterValue: "Code.S3Bucket is required when Code.S3Key is provided" |
| ZipFile and S3Bucket both set | InvalidParameterValue: "ZipFile and S3Bucket are mutually exclusive; provide one code source" |
| ZipFile or S3Bucket combined with ImageUri | InvalidParameterValue: "ImageUri cannot be combined with ZipFile or S3Bucket" |
| S3ObjectVersion without S3Bucket/S3Key | InvalidParameterValue: "Code.S3ObjectVersion requires Code.S3Bucket and Code.S3Key" |
| Empty S3 fields | InvalidParameterValue: "<field> must not be empty" |
| Oversize S3 fields | InvalidParameterValue: "<field> must be at most <N> bytes" (63/1024/256) |
| ZipFile only | unchanged (inline path) |
| S3Bucket only | new: fetch + store (section 3.3) |
| ImageUri only | unchanged |

Validation runs at the boundary (AGENTS.md section Safety & Security —
validate at deserialization/boundary, before business logic).

### 3.5 Error mapping

The provider converts S3CodeFetchError to AWS-compatible
LambdaServiceError::InvalidParameter messages:

| S3CodeFetchError | Lambda message (mirrors AWS) |
|---|---|
| BucketNotFound { bucket } | Error occurred while GetObject. S3 Error Code: NoSuchBucket. S3 Error Message: The specified bucket does not exist. |
| ObjectNotFound { bucket, key } | Error occurred while GetObject. S3 Error Code: NoSuchKey. S3 Error Message: The specified key does not exist. |
| VersionNotFound { .. } | Error occurred while GetObject. S3 Error Code: NoSuchVersion. S3 Error Message: The specified version does not exist. |
| Internal (S3 disabled) | Error occurred while GetObject. S3 service is not enabled. Enable S3 (SERVICES=s3,lambda) or provide code via ZipFile. |
| Internal (other) | Error occurred while GetObject. <source> |

All map to LambdaErrorCode::InvalidParameterValueException (HTTP 400), which
is what real AWS Lambda returns for S3 code fetch failures.

## 4. App bridge (apps/rustack)

New module apps/rustack/src/lambda_s3_bridge.rs following the sns_bridge.rs /
events_bridge.rs pattern:

```rust
/// Bridge between Lambda and rustack's in-process S3 provider.
///
/// Lives in the server binary to avoid a direct dependency from
/// rustack-lambda-core to rustack-s3-core.
#[derive(Debug)]
pub struct LambdaS3CodeFetcher {
    s3: Arc<RustackS3>,
    read_timeout: Duration, // DEFAULT_READ_TIMEOUT (30s)
}

#[async_trait]
impl S3CodeFetcher for LambdaS3CodeFetcher {
    async fn fetch_code(&self, bucket, key, version) -> Result<Bytes, S3CodeFetchError> {
        // 1. resolve bucket via S3ServiceState::get_bucket
        // 2. resolve object under the bucket's objects RwLock
        //    - version: Some(v) ──▶ get_version(key, v)  (reject delete markers)
        //    - version: None     ──▶ get(key)            (latest non-delete-marker)
        //    resolve the concrete storage version_id ("null" for unversioned)
        // 3. drop the lock, then storage().read_object(bucket, key, version_id, None)
        //    wrapped in tokio::time::timeout(read_timeout) so a stalled disk
        //    I/O cannot hold the request open indefinitely (gateway has no
        //    enclosing request timeout)
        //    (parking_lot guards are !Send — never held across .await,
        //     mirroring ops/object.rs handle_get_object)
    }
}
```

Failure mapping inside the bridge:

- get_bucket errors → BucketNotFound
- get_version / get misses → VersionNotFound / ObjectNotFound
- delete marker resolved (explicit version) → ObjectNotFound
- read_object NoSuchKey → ObjectNotFound; other → Internal

Version semantics match handle_get_object: unversioned buckets store under
version id "null"; versioned buckets use generated ids and get(key) returns
the latest non-delete-marker. S3ObjectVersion from the API request maps to an
explicit version lookup, exactly like GetObject?versionId=...

## 5. App wiring (apps/rustack/src/main.rs)

The S3 provider construction (currently between the Lambda block and the
CloudFront block) moves before the Lambda block so Lambda can reference it.
The Lambda block then selects a fetcher, mirroring the SNS/EventBridge
if-let-else-Noop pattern:

```text
┌─ main() service wiring ────────────────────────────────────────────────┐
│                                                                       │
│  #[cfg(feature = "s3")] let s3_provider_arc: Option<Arc<RustackS3>>   │
│      = if is_enabled("s3") { Some(RustackS3::new(cfg)) } else { None }│
│                                                                       │
│  #[cfg(feature = "lambda")] if is_enabled("lambda") {                 │
│      let fetcher: Arc<dyn S3CodeFetcher> =                            │
│          s3_provider_arc.as_ref()                                     │
│              .map(|s3| Arc::new(LambdaS3CodeFetcher::new(Arc::clone(s3)))) │
│              .unwrap_or_else(|| Arc::new(UnavailableS3CodeFetcher::default())); │
│      let provider = RustackLambda::new(cfg).with_code_fetcher(fetcher);│
│  }                                                                    │
│  ... CloudFront (shares s3_provider_arc as today) ...                 │
└───────────────────────────────────────────────────────────────────────┘
```

Feature-flag interplay handled by #[cfg(feature = "s3")] /
#[cfg(not(feature = "s3"))] bindings for the fetcher variable. When S3 is
compiled but disabled at runtime (SERVICES without s3), the unavailable
fetcher produces the clear error in section 4.5.

## 6. Testing Plan

| Layer | Test | Assertion |
|---|---|---|
| unit (rustack-lambda-core) | create_function with S3 code via mock fetcher | code_path/sha/size populated; record state Active; invoke record carries zip bytes |
| unit (rustack-lambda-core) | create_function, S3 bucket missing (mock returns BucketNotFound) | InvalidParameterValue with NoSuchBucket message; no record created |
| unit (rustack-lambda-core) | create_function, object missing | NoSuchKey message |
| unit (rustack-lambda-core) | create_function with ZipFile and S3Bucket | rejected, mutually exclusive |
| unit (rustack-lambda-core) | create_function S3Bucket without S3Key | rejected, S3Key required |
| unit (rustack-lambda-core) | update_function_code with S3 code | code_sha256/code_size updated on $LATEST |
| unit (rustack-lambda-core) | oversize S3 package (> 50 MB) on create and update | rejected with "Unzipped size must be smaller" before any storage |
| unit (rustack-lambda-core) | S3ObjectVersion without S3Bucket/S3Key | rejected, version requires a full S3 location |
| unit (rustack-lambda-core) | storage: zip expanding beyond 250 MB extraction budget | InvalidZipFile (zip-bomb guard) |
| unit (apps/rustack bridge) | delete-marker version id pinned | ObjectNotFound; original version still retrievable; latest gone |
| unit (rustack-lambda-core) | unavailable fetcher (default new()) with S3 code | clear "S3 service is not enabled" error |
| unit (apps/rustack bridge) | real RustackS3: put object → fetch | bytes match round-trip |
| unit (apps/rustack bridge) | missing bucket / key / explicit version | correct error variant |
| unit (apps/rustack bridge) | versioned bucket: latest vs explicit S3ObjectVersion | correct bytes each way |
| integration (tests/integration) | SDK CreateFunction with S3 code against running server | function created; GetFunction code size > 0; invoke round-trip (native executor) |
| integration (tests/integration) | SDK CreateFunction with missing bucket | InvalidParameterValueException |

Test naming per AGENTS.md section Testing (test_should_...). Mock fetcher via
a small test double in #[cfg(test)], not mockall — the seam is a single
trait method.

## 7. Exit Criteria

1. CreateFunction with S3Bucket/S3Key (object present in rustack S3)
   downloads, stores, and yields a function whose Invoke succeeds with the
   real bootstrap — proven by the native-executor integration test.
2. All invalid S3 references fail at creation time with
   InvalidParameterValueException and AWS-compatible messages (unit tests).
3. UpdateFunctionCode with S3 code updates code_sha256/code_size.
4. No core->core dependency: rustack-lambda-core has no rustack-s3-core
   dependency; the bridge lives in apps/rustack.
5. Hardening: S3 packages over the 50 MB zipped limit fail before any
   storage/extraction on both create and update; extraction is capped at
   250 MB unzipped; the S3 read is bounded by a 30s timeout; S3ObjectVersion
   requires a full S3 location.
6. Quality gates green: cargo build, cargo test, cargo +nightly fmt --check,
   cargo clippy -- -D warnings, doc build with RUSTDOCFLAGS="-D warnings".
7. Spec set updated: this design, specs/README.md index, and the stale
   "does not fetch from S3" notes in rustack-lambda-design.md /
   rustack-lambda-executor-design.md all reflect the new behavior.

## 8. Future Work (deferred)

- **HTTP-backed fetcher** for external S3 endpoints (LocalStack, MinIO, real
  AWS): implement S3CodeFetcher over an HTTP client + SigV4 (reusing
  rustack-auth), selected by a LAMBDA_S3_ENDPOINT-style config. The provider
  needs zero changes.
- **Layer version S3 code** for PublishLayerVersion (separate gap).
- **Code signing config enforcement**.
