# Workflows 93: Implementation Review and Required Follow-ups

Status: active implementation record · Date: 2026-10-25

This is the canonical review backlog for P0–P5. A corrected specification is not evidence that its runtime consumer has been implemented. All required in-scope entries must close before the complete feature is declared done.

## P0 independent review

| ID | Priority / source | Finding and resolution | Remaining implementation gate |
|---|---|---|---|
| W01 | P1; `apps/rustack/src/main.rs:373`, runtime transport contract | Bollard logs request/response/attach bytes at debug/trace. Specification now requires an independent mandatory dependency filter, even with RUST_LOG=trace | P1: implement filter and captured-log secret/payload regression |
| W02 | P1; runtime protocol contract | BootstrapReady was missing from the message variants despite being required by initialization. Specification fixed; real relay probe uses the separate signal | P1: implement production typed signal and initialization tests |
| W03 | P1; runtime create contract | Omitted Architectures must default to x86_64, unlike an explicit empty list. Specification corrected | P1: normalize before validation and cover omission/empty/multiple |
| W04 | P1; `codegen/src/shapes.rs:180`, `codegen/src/codegen.rs:263` | Generated chrono fields do not accept numeric AWS JSON timestamps. Independently reproduced; required/optional/fractional/negative codec behavior pinned | P2 task2.1: generator fix and executable generated round trips before service generation |
| W05 | P2; callback error contract | Known completed/revoked callback tasks use TaskTimedOut, not TaskDoesNotExist. Specification and fixtures corrected | P4: implement token tombstone classification and retry/cancel cases |
| W06 | P1; pinned AWS-validated `test_callback.snapshot.json:1251` | `.sync:2` decodes Input as well as Output. Root's subsequent oracle inspection corrected the specification and added TaskSubmitted | P3: validate complete optimized envelope, dates and history sequence |

All six changes preserve the intended scope. No review finding has been dismissed as invalid. P0 probe review produced the following additional fixes.

## P0 retained-probe review

| ID | Priority | Finding and fix | Evidence |
|---|---|---|---|
| W07 | P2 | Create-response cancellation can lose an ID. Allocate a run UUID before requests, label every resource, reconcile labels under a bounded cleanup deadline, and deliberately omit one create result from the ID registry | Both retained architecture runs report labelReconciliation=true and independent daemon queries find no P0 containers |
| W08 | P2 | Close/Ack did not establish EOF behavior. Add clean/truncated-prefix/truncated-body EOF tests and compare real attach-input shutdown with StdinOnce false/true | Seven unit tests pass; both architecture runs show false leaves the relay running and true exits before cleanup. Production spec now requires true |
| W09 | P3 | Offline Python tests generated untracked bytecode | Probe target sets PYTHONDONTWRITEBYTECODE=1; generated test bytecode removed and excluded from staged files |

The independent final recheck confirmed W07–W09 addressed, queried both daemon run labels and found zero containers, and reported no further in-P0 correctness issue. Production consumers remain subject to W01–W06.

## P1 task1.1 foundation slice review

The independent read-only review against commit `955267f` found no new in-slice correctness, security or compatibility defect. It verified strict resource UUID deserialization, immutable lifetime identity, legacy and current snapshot behavior, duplicate identity rejection, complete configuration capture, alias metadata versus resolved version, warm-key isolation and request/result diagnostic redaction. Ten new regression tests pass within 154 Lambda core tests. This approval covers the capture foundation only; artifact enums, cooperative cancellation and Docker execution remain required in P1. Standard workspace build/test/Clippy/rustdoc and two identity doctests pass; the native target validates six native/DryRun cases (its separately gated Squib case is not execution evidence). Baseline advisory remediation B01 and strict-lint remediation B03 remain open.

## P1 task1.1 artifact slice review

The independent review against commit `7915281` identified two implementation findings and one preflight-order documentation mismatch. All were fixed and the final independent recheck found no remaining in-slice issue.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W10 | P2; `crates/rustack-lambda-core/src/provider.rs:504`, `src/storage.rs:74` | Rejected settings could retain a staged ZIP. Shared settings preflight now runs before source IO. Unpublished-artifact guards roll back on failure/cancellation/CAS conflict, and restore commits guards only after the complete candidate validates | Invalid settings leave no artifacts; real revision-conflict, aborted owner and rejected restore regressions preserve committed resources |
| W11 | P3; `crates/rustack-lambda-core/src/artifact/image.rs:141` | Docker classifies uppercase first components as registry domains. Parser and cases now accept Registry/worker while rejecting uppercase repository components | Parameterized reference and serde cases cover single-label registries, DNS/IPv6 ports, tags/digests and invalid syntax |
| W12 | P3; `crates/rustack-lambda-core/src/storage.rs:813` | Snapshot settings validation followed staging despite the documented ordering. Shared settings preflight now runs before archive decoding or staging; full integrity checks still retain rollback ownership | Early-invalid-settings/invalid-base64 regression plus a separate post-staging hash-mismatch rollback regression |

The slice also preserves extracted ZIP roots in Native warm keys and retains code in Native child supervision through exit. The standard workspace suite passes 1,752 tests (392 preexisting ignored), including 203 Lambda core tests; twelve Lambda doctests and six native/DryRun cases pass. The separately gated Squib case selected by the native Make target is not Squib execution evidence. New winnow/rstest dependency use introduces no audit finding; the same 16 baseline advisories remain B01. Strict production boundary lint debt remains B03; no ignore was added. ExecutionRevision ownership, cooperative invocation cancellation, deployment resolution and Docker remain required before P1/M0 completion.

## P1 task1.1 cancellation/revision slice review

The independent read-only review found three lifecycle/admission defects. All were fixed; its final recheck found no further concrete in-slice finding.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W13 | P1; `crates/rustack-lambda-core/src/executor/squib.rs:170` | Dropping an idle Squib executor relied on the facade's best-effort Drop and detached VMM handle. OwnedVm now transfers idle/busy VM shutdown into retained CleanupTask, using the captured parent Tokio handle | Generic owned-VM seam test drops off-runtime, expires a cleanup waiter and proves allocation retention until shutdown completes. This is not actual VM execution evidence |
| W14 | P2; `crates/rustack-lambda-core/src/executor/instance.rs:276` | A full/disabled warm pool dropped a successful instance before reaping. Release now returns rejected ownership; Invoke awaits API shutdown and backend termination before settlement | Two overlapping real Native calls with warm retention0/1 require exactly0/1 surviving child after both responses |
| W15 | P2; `crates/rustack-lambda-core/src/executor/native.rs:129` | Known host-architecture mismatch failed only after Event acceptance. Availability now checks architecture before work admission | Real fixture Event rejects the mismatch, then quiesces cleanly without starting the foreign bootstrap |

ExecutionRevision uses shared Arc ownership; InvocationHandle and the bounded work actor preserve UUID/deadline/cancellation through backend cleanup. Waiter cancellation never aborts backend workers. Native child supervisors retain artifact leases and process reservations through reaping, including executor Drop. Tests cover late-result rejection, cleanup timeout/aborted waiters, worker panic,128 accepted/32 running/default8 function limits and permit release after cleanup. The workspace suite passes1,767 tests (392 preexisting ignored), including215 Lambda core tests and five real Native fixture tests. Lambda doctests, standard workspace build/Clippy/rustdoc and the native SDK target pass. Its separately gated Squib test remains excluded as VM evidence. Full Docker and workflow milestones, plus B01/B03, remain required.

## P1 task1.2 protocol slice review

The independent read-only review verified bounded preflight before Serde, typed variants, duplicate/unknown-field rejection, independent metadata/payload budgets, canonical identities, redacted diagnostics, absolute IO deadlines and clean/truncated EOF handling. Its final recheck found no remaining in-slice finding.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W16 | P3; `crates/rustack-lambda-runtime/src/protocol/types.rs`, FunctionArn parsing | Numeric qualifier zero/overflow differed from the provider's validated qualifier. Runtime ARN parsing now enforces the same positive-u64 rule | Direct parse and full envelope deserialization reject0 and18446744073709551616 |

The shared runtime crate has39 passing codec tests, including512-case properties for malformed bytes, mutated valid envelopes and arbitrary binary payloads, plus eight passing doctests. The workspace suite passes1,806 tests (392 preexisting ignored); build/fmt/Clippy/rustdoc and strict runtime production-boundary checks pass. `make test-lambda-runtime` runs the reusable contract suite. Proptest1.12 uses only std; its three added dev packages introduce no audit finding. The same16 B01 advisories remain; deny bans/licenses/sources pass. This approval covers the protocol only: shared RuntimeSession, HTTP migration, relay executable, Docker execution and workflow engine remain required.

## P1 task1.2 shared-session slice review

The independent read-only review found three cancellation/order races. All were fixed and the final recheck found no remaining concrete in-slice finding.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W17 | P2; `crates/rustack-lambda-runtime/src/session.rs:821` | A sent oneshot response was treated as delivered before its waiter consumed it. Offering now retains Invocation until a per-poll receipt; abandoned receipts recover queued ownership | Deterministic manual-poll test drops next() after send but before consumption, then receives the same event exactly once through a replacement poll |
| W18 | P2; `crates/rustack-lambda-runtime/src/session.rs:782` | Biased selection did not recheck cancellation arriving between branch polls. Offer/receipt/completion and command dispatch now revalidate cancellation, owner closure and deadline before mutation | Direct transition tests cover cancelled/closed/expired attempts across offer, receipt and completion and require Closed with failed pending results |
| W19 | P2; `crates/rustack-lambda-runtime/src/session.rs:768` | A receipt and following completion could arrive between select branch polls, leaving stale Offering/initialization state. Command dispatch now synchronously reconciles ready/closed receipts before matching commands | Ready-receipt→Complete succeeds; ready-receipt→InitError returns Busy and preserves the delivered attempt |

The actor owns an eight-command mailbox, one queued/offered/delivered attempt and one poll; per-attempt tickets prevent old cancellation from targeting new work. RPC envelopes carry their own deadline, readiness changes once, initialization notifies once, shutdown retains its join handle across cancelled waiters and caches failed settlement. Sixteen actor tests and the39 codec tests pass;23 runtime doctests and strict production-boundary Clippy pass. Workspace build/fmt/Clippy/rustdoc and1,822 tests pass (392 preexisting ignored). This review covers the shared actor only: the Native HTTP adapter still uses its existing session implementation until migration; the relay/socket client, Docker executor and ASL engine are not implemented by this slice. B01/B03 remain required.

## P1 task1.2 shared HTTP / Native migration review

The independent read-only review found three boundary/lifecycle defects. All were fixed and the final recheck found no remaining concrete in-slice finding.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W20 | P1; `crates/rustack-lambda-runtime/src/http.rs:216`, pinned Hyper1.11.1 `proto/h1/decode.rs:149` | Hyper TRACE includes partial trailer bytes before application validation. Every HTTP supervisor/connection future now uses an independent subscriber denying Hyper/hyper_util DEBUG/TRACE while delegating allowed application context/events | TRACE capture covers ordinary target/header/body and a deliberately fragmented secret trailer; no secret reaches diagnostics |
| W21 | P2; `crates/rustack-lambda-runtime/src/http.rs:456` | Collected trailers bypassed per-header validation. All trailers now reject before session mutation | Short, oversized and Invocation-Id trailers return400 without initialization notification or session closure |
| W22 | P2; `crates/rustack-lambda-core/src/executor/instance.rs:309` | Revision mismatch cleared old instances through Drop, losing API join ownership. Acquisition now extracts stale instances and awaits bounded parallel retirement outside map guards; empty buckets are removed | Real old API holds an incomplete body while code changes; replacement waits its408 deadline/join and old child reap before invoking new code |

The shared adapter replaces the old pending-map implementation while preserving executor module reexports. Native cold start now races first-poll readiness, init error, process exit and caller cancellation under one init deadline, retiring API/process owners on failure. The Native executor's one cleanup task atomically captures reaper/pool/child ownership, survives cancelled/concurrent shutdown waiters and preserves cleanup failures. Eleven real fixture tests cover slow/no polling, init failure, startup cancellation, version isolation, stale connections, busy cancellation and joined shutdown. The existing native SDK target passes six Native/DryRun scenarios; its separately gated Squib selection remains excluded as VM execution evidence.

Workspace build/fmt/Clippy/rustdoc and1,841 tests pass (392 preexisting ignored). Shared runtime:71 tests,34 doctests and strict production-boundary Clippy; Lambda core:212 tests and26 doctests. Three old core HTTP tests moved into the shared adapter's stronger wire suite. Cargo audit still finds the same16 B01 advisories; no new dependency version/package landed, and deny bans/licenses/sources pass. B03 remains open for old core boundaries. The relay executable/client, Docker executor and ASL engine remain required before milestone completion.

## Existing baseline issues affecting final gates

| ID | Priority / source | Verified evidence | Required disposition |
|---|---|---|---|
| B01 | P1; `Cargo.lock:1462`, `:1474`, `:4308`, `:5893` | Baseline cargo audit finds 16 advisories across crypto-bigint 0.5.5/0.7.5, ctutils, quinn-proto, rustls/webpki roots and ZIP 8.6.0. cargo-deny fails its advisory gate; bans/licenses/sources pass. Standalone probe audit is clean | P1 dependency work: upgrade affected dependencies, including ZIP major and the SDK's SigV4a chain; rerun meaningful regressions and audit/deny. Do not add ignores to make the gate green |
| B02 | P2; `crates/rustack-kms-model/src/epoch_seconds.rs:39` | Existing optional epoch helper truncates negative fractional seconds and casts a negative fraction to u32; it is unsuitable as the generator's new codec | Do not reuse it in P2. Existing KMS correction is outside this workflow feature; retain as a repository follow-up without claiming it fixed |
| B03 | P2; `crates/rustack-lambda-core/src/executor/native.rs:336`, `src/resolver.rs:271`, `src/storage.rs:1292` and existing provider Errors docs | Strict production boundary Clippy on lambda-core reports 66 existing findings: 56 missing Errors sections, four documentation-format findings and six indexing/slicing sites. The unfiltered dependency run additionally fails existing rustack-auth/core/model findings. The new identity/request/pool code has no finding in the strict core run; standard workspace Clippy passes | P1 quality work: fix the affected production boundaries and documentation, then rerun strict checks without lint ignores; standard checks alone do not close this gate |

The P0 record distinguishes unchanged workspace advisory failures from the clean probe dependency graph; it never reports a fully green security baseline.

## Cross-references

- ← [implementation plan](./rustack-workflows-90-impl-plan.md)
- ↔ [runtime spike](../docs/research/spike-lambda-container-relay.md), [semantic oracle](../docs/research/spike-stepfunctions-semantic-oracle.md), [verification gates](./rustack-workflows-70-verification-plan.md)
