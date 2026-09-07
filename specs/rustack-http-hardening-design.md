# HTTP trust boundaries and IO hardening design

Implements [system review](rustack-system-review.md) R02/R03/R04 and service-side R05; gateway connection/admission and operator YAML wiring belong to the root integrator. Budgets follow [implementation plan](rustack-system-hardening-impl-plan.md) §3. Prior art: [S3 streaming research](../docs/research/s3s-crate-research.md), [checksum protocol](s3-checksum-parity-design.md).

## Request pipeline and ownership

```text
Gateway: validated config → connection/request admission (root owner)
                                      │
HTTP protocol: route → resolve AuthMode → bounded frame reader
                                      │       ├─ byte overflow → protocol error, drop source
                                      │       └─ idle/total deadline → error, drop source
                                      ▼
                           actual payload SHA256 → signature verification
                                      ▼
                              decode → handler → response
Proxy: configured URL → no redirects/no environment proxy → bounded upstream frames
                                      └─ failure cancels response, never inserts partial cache
```

No business handler executes after auth/body validation failure. Early source enforcement is mandatory: Content-Length is only an optimization, never the authority. Every DATA frame is checked before copying into an aggregate. Transport failures are propagated, not interpreted as empty EOF. Cancellation drops the owned source; no detached reader tasks.

## Authentication API and compatibility

`rustack-auth::AuthMode` expresses `Development` or `Required(&dyn CredentialProvider)`; resolving legacy `(skip_signature_validation, Option<provider>)` rejects strict+None. Keep existing public service config fields for source compatibility; root startup must reject the same invalid combination before listening. All 19 HTTP adapters, including formerly ignored CloudFront config, use this rule. Strict S3 anonymous requests are rejected; presigned and SigV2 remain explicit S3-only paths.

Ordinary `verify_sigv4` accepts an actual body digest, never a client-selected digest. At most one x-amz-content-sha256 header is allowed; concrete values are exactly 64 lowercase hexadecimal bytes and equal the digest of the received bytes. Missing hash headers use actual digest. Duplicate Authorization headers are rejected. No Authorization, expected signature or supplied signature is emitted to tracing/errors. The separately named `verify_s3_sigv4` entry point allows exact `UNSIGNED-PAYLOAD` and authenticates seeds for `STREAMING-AWS4-HMAC-SHA256-PAYLOAD`, its `-TRAILER` variant, and `STREAMING-UNSIGNED-PAYLOAD-TRAILER`. `StreamingVerifier` verifies each SHA256/HMAC chain element (including zero-size terminal chunk) and the canonical declared trailer block before publication. Unknown markers, including previously accepted but never cryptographically implemented SigV4a/ECDSA markers, fail closed rather than claiming verification. This is not a blanket removal of streaming support. S3 unsigned trailer encoding must decode and validate declared checksums before publication; generic JSON/Query adapters reject all placeholders.

## IO API and budget matrix

Shared `rustack-core::http` owns a validated `BodyBudget`, a `BudgetedBody<B>` (actual DATA byte count, absolute total deadline, reset-on-progress idle deadline), and `collect_body`. Validation rejects zero limits/durations. Default control-plane budget is 16 MiB / 30 s total / 5 s idle. Lambda code JSON envelope is 96 MiB; invoke requests use the existing synchronous/asynchronous service limits and cannot inherit the code-upload allowance. S3 XML/control-plane remains 16 MiB. Object upload paths require 5 GiB streaming with bounded frames/storage, not a 5 GiB `Bytes` allocation. S3 integration now stages PutObject and UploadPart with `UploadWriter`/`StagedUpload`: private RAII tempfile, incremental MD5/CRC32/CRC32C/CRC64NVME/SHA1/SHA256, and 64 KiB disk read buffers. The raw encoded stream has a separate ceiling of twice decoded capacity plus 16 MiB for framing; decoded writes enforce the exact configured object ceiling. Chunk lines are capped at 8 KiB; at most eight allowlisted trailers, declared exactly once, no duplicate header/trailer checksum. The S3 total deadline defaults to one hour with 5 s idle; control total remains 30 s. The HTTP reader enforces actual bytes before the staging writer, not after receiving a full body.

Upstream response default is 64 MiB, 30 s total and 5 s idle. Reqwest clients disable redirects and environment proxy inheritance and set connect/request deadlines. Readers use `Response::chunk`, check remaining capacity before append and abort immediately on overflow; advertised lengths do not replace accounting. CloudFront's configured smaller response limit remains authoritative. APIGW integration timeout may tighten but never remove the default deadline. Ordinary control responses use protocol-native error envelopes. Budget errors must not be emitted as successful responses.

## Proxy URL/protocol policy

Only operator-configured integration/origin determines initial authority. Parse URLs, allow HTTP/HTTPS only, disallow credentials/fragments; request path cannot replace authority. Local HTTP and private/loopback destinations are intentional local-emulator operator configuration exceptions, not arbitrary viewer-selected egress. Do not follow any 301/302/303/307/308 or relative/protocol-relative Location. Forward original status and Location unchanged. Never inherit HTTP_PROXY/HTTPS_PROXY/ALL_PROXY. This closes redirect destination expansion without claiming a general DNS-pinning/remote-host isolation feature.

## Verification and exit evidence

- R02: every adapter strict+None rejects before handler; required credentials accept correctly signed request and reject missing/malformed signatures. Development remains explicit.
- R03: known signing vectors; changed payload; injected unsigned hash header; malformed/duplicate hash headers; absent header; ordinary placeholders rejected; S3 exceptions are narrow; tracing has no signature material.
- R04: loopback A returns each 30x pointing to B (absolute, relative, protocol-relative, IPv6); B receives zero requests and response preserves Location; no second response cached. Client construction statically requires no-proxy/no-redirect.
- R05: B bytes pass, B+1 bytes fail during frame read even without Content-Length; oversized frame is not copied; pending source reaches idle/total deadline; progress cannot reset total deadline; dropping read releases source. Slow/oversized upstream rejected before aggregate growth exceeds budget. S3 streaming tests cover staging cleanup, range download chunk sizes, bad-checksum overwrite preservation and incremental multipart assembly.
- Targeted cargo checks/tests use managed background jobs; root owner runs workspace gates and maintains Cargo.lock/Makefile/apps. Independent review is performed by root owner (this worker must not delegate).

## Integration changes and evidence log

Shared helper adds `pub mod http;` to rustack-core. Dependency additions only reuse workspace crates and were coordinated with the root owner. No gateway/main changes are owned here. Root settings supplies validated body/upstream byte, idle, total and S3-specific budgets; `BodyBudget` constructors consume them. `append_bounded` caps buffer capacity growth, not only used length. Native protocol error envelopes preserve 413 for overflow, 408 for idle/total timeouts, and 400 for broken transport.

### S3 publication and public integration contract

```text
Incoming DATA ─► encoded byte/deadline budget ─► AWS chunk parser / HMAC chain
                                                  │ bounded pieces, no full object
                                                  ▼
                                       UploadWriter (private tempfile)
                                                  │ finish + all digest/trailer checks
                                                  ▼
                              Arc<StagedUpload> ─► S3Handler::handle_staged_upload
                                                  │ core validates MD5/checksum/metadata
                                                  ▼
                                           storage publication
                                                  │ immutable shared file + metadata
                          GetObject ─► StagedRead ─► bounded FileBody ─► viewer
```

`S3Handler` adds `handle_staged_upload(parts, Arc<StagedUpload>, ctx)` with a rejecting default, so an old custom buffered handler never receives an empty upload disguised as a valid request. The application bridge implements the method; generated model blobs stay unchanged. Core adds staged PutObject/UploadPart and streaming GetObject entry points. S3 HTTP adds `S3ResponseBody::Streaming` and `from_staged`; immutable file ownership remains alive through response completion. CopyObject and whole-object UploadPartCopy reuse immutable staged data; multipart assembly copies bounded pieces to a new staged artifact rather than concatenating all parts in RAM. HTTP->S3-core dependency is intentionally limited to this existing storage/checksum domain contract, as CloudFront's HTTP layering already depends on its core. There is no core->HTTP edge or cycle; moving the S3-specific checksum/tempfile contract into shared rustack-core would unnecessarily add all S3 crypto/storage dependencies to every service. An external custom S3Handler now needs the new method for uploads; this explicit compatibility change was sent to the root integrator.

APIGW `RustackApiGatewayV2::new` returns `Result<Self, ApiGatewayV2ServiceError>` because constructing the isolated client is fallible and must not panic/fallback. CloudFront `dispatch_s3_origin` accepts a final `max_body` argument and rejects a staged object exceeding the proxy cap before reading the file. Root main/API constructor calls and gateway ingress use these contracts.

### Verification log

- First auth/core unit run: 42/43 auth passed; missing Authorization incorrectly mapped to InvalidAuthHeader. Corrected duplicate detection to reject count >1, preserving MissingAuthHeader.
- Early S3/auth/core targeted `cargo check`: passed.
- Two test/clippy attempts observed root-owned settings.rs mid-edit (missing module/delimiter and aliases); no worker changes to root-owned code were made.
- One cargo attempt was sandbox-denied during concurrent rustup self-update cleanup. Root owner completed the external cleanup and reran the exact command; no alternate cargo path or permission workaround was used.
- Root-assisted auth46/core16 passed; S3-core missing-part variant regression corrected from Internal to InvalidPart.
- S3-core loopback-default provider test updated consistently with root-approved safe default.
- Managed `bash-64`: `cargo test --lib` for all 18 *-http crates plus auth/core/S3-core/CloudFront-dataplane/APIGW-core passed. New regressions include SQS handler-not-called, tampered/injected hashes, signed S3 chunk corruption, unsigned trailer checksums, 17 MiB staging in 64 KiB frames and temp-file deletion, all 30x classes with absolute/relative/protocol-relative/IPv6 targets, chunked proxy overflow and never-EOF upstream deadline.
- Targeted nightly rustfmt completed successfully (explicit owned paths, not workspace formatting).
- Later targeted clippy exposed moved legacy operation allowances and a new long upload parser; the legacy annotations were moved with their existing code, and the new parser was factored into bounded protocol stages rather than suppressed. Final gate results are appended at handoff.
