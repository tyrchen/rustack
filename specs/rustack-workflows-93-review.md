# Workflows 93: Implementation Review and Required Follow-ups

Status: active implementation record · Date: 2026-10-25

This is the canonical review backlog for P0–P5. A corrected specification is not evidence that its runtime consumer has been implemented. All required in-scope entries must close before the complete feature is declared done.

## P0 independent review

| ID | Priority / source | Finding and resolution | Remaining implementation gate |
|---|---|---|---|
| W01 | P1; `apps/rustack/src/main.rs:373`, runtime transport contract | Bollard logs request/response/attach bytes at debug/trace. Specification now requires an independent mandatory dependency filter, even with RUST_LOG=trace | Closed for implemented runtime/wire paths: mandatory task-bound filter plus captured Hyper and actual Bollard call-site regressions. Future Docker stream consumers must retain the same policy |
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

ExecutionRevision uses shared Arc ownership; InvocationHandle and the bounded work actor preserve UUID/deadline/cancellation through backend cleanup. Waiter cancellation never aborts backend workers. Native child supervisors retain artifact leases and process reservations through reaping, including executor Drop. Tests cover late-result rejection, cleanup timeout/aborted waiters, worker panic,128 accepted/32 running/default8 function limits and permit release after cleanup. The workspace suite passes1,767 tests (392 preexisting ignored), including215 Lambda core tests and five real Native fixture tests. Lambda doctests, standard workspace build/Clippy/rustdoc and the native provider target pass. Its separately gated Squib test remains excluded as VM evidence. Full Docker and workflow milestones, plus B01/B03, remain required.

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

The shared adapter replaces the old pending-map implementation while preserving executor module reexports. Native cold start now races first-poll readiness, init error, process exit and caller cancellation under one init deadline, retiring API/process owners on failure. The Native executor's one cleanup task atomically captures reaper/pool/child ownership, survives cancelled/concurrent shutdown waiters and preserves cleanup failures. Eleven real fixture tests cover slow/no polling, init failure, startup cancellation, version isolation, stale connections, busy cancellation and joined shutdown. The existing native provider target passes six Native/DryRun scenarios; its separately gated Squib selection remains excluded as VM execution evidence.

Workspace build/fmt/Clippy/rustdoc and1,841 tests pass (392 preexisting ignored). Shared runtime:71 tests,34 doctests and strict production-boundary Clippy; Lambda core:212 tests and26 doctests. Three old core HTTP tests moved into the shared adapter's stronger wire suite. Cargo audit still finds the same16 B01 advisories; no new dependency version/package landed, and deny bans/licenses/sources pass. B03 remains open for old core boundaries. The relay executable/client, Docker executor and ASL engine remain required before milestone completion.

## P1 task1.2 Linux relay slice review

The independent read-only review found one documentation mismatch and two test-harness ownership gaps. A self-review also found a completed-worker error masking race; its fix was independently rechecked. The final review found no remaining concrete in-slice issue.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W23 | P3; `specs/rustack-workflows-20-lambda-runtime-design.md` §3.4 | Retiring was described as Close-only while duplicate matching Cancel was deliberately idempotent. The contract now documents that exception without refreshing the lease, and distinguishes fatal session/attempt expiry from the bounded retirement handshake | Repeated busy/post-completion Cancel tests plus explicit real RIC cancellation qualification |
| W24 | P2; self-review, `crates/rustack-lambda-runtime/src/relay.rs`, RelayActor::run | Priority stop could win while a completed IO error remained in JoinSet; teardown ignored that error. Drain now preserves completed IO failure/unexpected writer exit and ignores only joined abort cancellation or the expected queued-Close reader exit | Deterministic pre-completed EOF worker with priority stop requires EOF failure and a closed API listener |
| W25 | P2; `crates/rustack-lambda-runtime/tests/docker.rs`, busy-marker loop | An outer timeout could cancel a CLI helper before explicit kill/reap. Forward one absolute deadline into the helper owner instead | Real docker wait helper expires and is reaped while the relay remains usable for Close/Ack |
| W26 | P2; same file, Environment::cleanup | A CLI reaping timeout returned before independent label reconciliation. Record cleanup errors, retain process ownership through reaping and still perform the label check before combining results | Every successful/failed setup, scope, pipe and handler case verifies no run-labeled container remains; final independent review confirms error aggregation order |

The relay binary uses safe nonblocking owned Linux pipe descriptors, task-bound tracing, bounded two-frame mailboxes and confirmed flush receipts. It separates relay Ready from first-RIC-poll BootstrapReady, retains borrowed attempt tickets across select cancellation, rejects invalid scope/order/direction, distinguishes runtime error payloads, and joins the shared API and control workers on every exit. Close and cancellation do not claim external function termination. Pinned provided.al2023 relay images are built by Make targets for both supported Linux architectures.

Workspace build/fmt/Clippy/rustdoc and1,853 tests pass. The ordinary suite has396 ignored tests:392 preexisting resource tests plus four explicitly gated Docker relay tests. Runtime:83 unit tests,39 doctests and strict production-boundary Clippy pass; the actual Linux entry point also passes cross-target Clippy. `make test-lambda-runtime-docker` explicitly ran all four tests with zero ignored on each arm64 VZ and amd64/QEMU daemon (eight executed test cases overall), proving official Python RIC computed42/7, warm counter1/2, request metadata, nonroot/read-only/cgroup limits, handler error, busy cancellation, Close/EOF/truncated/partial/idle exits, failed allocation and CLI helper timeout cleanup. Both daemons have zero remaining test-labeled containers. Native x86, Desktop and rootless qualification remain M4.

No dependency version/package was added; existing anyhow/serde_json are test dependencies and tracing-subscriber is also used by the relay entry point. Audit/deny report the same16 B01 advisories; bans/licenses/sources pass. B03 remains open. The production host control client, Docker deployment/pool/provider integration and ASL engine remain required; this is not full P1/M0 acceptance.

## P1 task1.2 host client / shared stream ownership review

The independent read-only review found two protocol-validation defects. Both were fixed and the final recheck found no remaining concrete in-slice finding. Self-review also corrected an error-class race exposed by the existing partial-frame regression after extracting stream ownership.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W27 | P2; `crates/rustack-lambda-runtime/src/client.rs`, ControlClient::next | Tokio timeout can poll an already-ready inner receive before its expired timer. Recheck the absolute clock after await and before frame/state mutation; shared send also rechecks after its flush receipt | Paused-clock pending waiter, capacity-one input and a queued valid frame reject after caller expiry; no false BootstrapReady |
| W28 | P3; same file, InitError handling | InitError was accepted after an earlier result proved delivery. Track proven delivery, including a discarded late cancellation result, and reject later initialization failures | Ordered Idle, later Busy and both cancellation-state cases fail closed after successful delivery |
| W29 | P2; self-review, `crates/rustack-lambda-runtime/src/transport.rs`, Transport::recv | Reader drops its sender before supervisor settlement, so an empty mailbox could report EOF before the actual parser/deadline failure. Wait for retained supervisor classification on channel closure | Existing paused partial-frame regression still requires Protocol/Deadline; completed EOF/priority-stop test preserves EOF and joined API closure |

The host validates ordered Hello/architecture/Ready and first-poll BootstrapReady, owns one active attempt, rejects foreign/duplicate/out-of-order events and discards at most one matching late outcome while cancelling. A write-in-progress flag prevents reuse after a dropped send future. Closing preserves validation of queued results/bootstrap/cancel acknowledgements; Ack/close cannot precede outstanding Cancel Ack. Cancelled next/shutdown waiters retain parsing and joined IO ownership. The same private transport supervisor now owns both relay and host stream workers, bounded mailboxes, terminal-frame preservation, flush receipts and cached failure settlement; the relay still separately joins its API before transport retirement.

Workspace build/test/fmt/Clippy/rustdoc pass with1,867 tests and397 ignored (392 preexisting plus five explicit Docker tests). Runtime:97 tests,47 doctests and strict production-boundary Clippy pass; the Linux entry point passes cross-target Clippy. All five resource-gated Docker tests explicitly pass with zero ignored on arm64 VZ and amd64/QEMU. The new case transfers actual CLI pipes into production ControlClient, performs computed42/7 and warm1/2, then tests separate real FunctionError and busy Cancel paths, joins streams, removes containers, reaps the CLI and reconciles its run label. This qualifies client/relay composition, not the future production Docker attach/deployment/pool/provider.

No dependency changes landed in this slice. The preceding audit remains unchanged B01 evidence; B01/B03 remain mandatory P1 gates. Docker startup/resource probes, production lifecycle integration and the ASL engine remain required by the complete P0–P5 objective.

## P1 task1.3 transport design preflight

The pinned [source study](../docs/research/study-bollard-docker-transport-boundaries.md) and runtime §3.6/D19 refine the forthcoming Docker consumer before production code. Independent read-only review confirmed the response collection, timeout, attach decoder and diagnostic facts. These are design/source checks, not runtime acceptance.

| ID | Priority / source | Finding and resolution | Remaining gate |
|---|---|---|---|
| W30 | P2; runtime §3.6 capacity | An eight-connection limit was ambiguous when32 environments each retain control/log streams. Management now uses six business/two reserved cleanup slots and32/8 mailboxes; supervised stream handoff releases management slots and retains independent environment/deployment reservations | Implement saturation/cleanup-lane tests and prove long-lived streams cannot starve kill/remove |
| W31 | P3; source study citations | Several references named enclosing methods instead of exact collection/length/trace expressions. Corrected to the vendored release's verified lines | Source/link/pin checks; no runtime behavior claim |

The final independent review found no remaining load-bearing design gap. W01 remains a mandatory consumer/filter/TRACE gate under the refined model-only production path. No Rust production/dependency source changed in this preflight; bounded fake-daemon and actual Docker wire/large-payload evidence remain required before the consumer lands.

## P1 task1.3 shared JSON preflight review

The independent read-only review found no concrete in-slice defect. It verified per-object decoded-key uniqueness, escaped keys, UTF-8 byte limits, encoded-string scratch bounds, aggregate member/element counting, root/container depth, malformed/trailing JSON, schema-error redaction and controlled key-limit configuration. The final recheck confirmed the generic parser does not hardcode Docker's256-byte key ceiling; consumers explicitly choose that tighter profile.

rustack-core::json now exposes validated nonzero JsonLimits, validate and from_slice<T>. A linear encoded-string scan precedes DeserializeSeed traversal; the traversal retains only bounded object key sets and builds no dynamic Value tree. Typed materialization follows successful preflight, and domain validation still belongs to the service. Core now forbids unsafe code. Four existing core documentation-format findings were corrected so its complete strict production-boundary check passes without ignores; B03's Lambda/auth/model debt remains open.

Core has39 passing tests (21 new JSON cases, including two512-case properties) and four passing doctests. `make test-json-boundaries` runs that suite and strict production Clippy. Workspace build/test/fmt/Clippy/rustdoc pass with1,888 tests and397 ignored resource tests. Existing proptest/rstest versions are reused only for core development; no dependency version/package was added. Audit/deny still report16 B01 advisories, with bans/licenses/sources passing. Docker metadata/stream consumers and all full P1/M0 exits remain required; a passing parser is not executor acceptance.

## P1 task1.3 bounded Docker wire review

The independent read-only review and final recheck found no remaining concrete in-slice issue after the lifecycle-wait correction below. Review covered typed operation paths and status handling, version negotiation, bounded serialization/JSON and HTTP framing, retained driver/handoff ownership, deadline/cancel races, worker panic settlement, strict multiplexed streaming and secret capture. No production DockerExecutor/provider acceptance is claimed.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W32 | P2; `crates/rustack-lambda-core/src/docker/mod.rs`, request admission | Long-lived Wait jobs could fill both reserved cleanup slots and prevent Kill/Remove. Dedicated bounded32-job/eight-mailbox wait lane now leaves two slots for inspect/kill/remove/list; six short business slots remain separate | Held-two-waits regression completes Kill before either wait is released; business saturation admits Remove; exported attach survives wire shutdown without retaining a management slot. D20 records the ownership rule |

DockerWire and DockerPort implement bounded Unix management and strict upgraded attach using generated model-only bollard-stubs1.53.1-rc.29.3.1. All15 new fake-daemon/framing tests pass within227 Lambda core tests. The workspace passes1,903 tests (398 explicitly resource-gated), build/fmt/Clippy/rustdoc; the unchanged strict Lambda boundary debt remains B03, with no new Docker-path findings. Runtime contracts pass97 tests and48 doctests plus strict production checks and actual Linux entry-point Clippy. Eleven real Native fixture tests pass; the native provider target passes its six actual native/DryRun cases, with its separately gated Squib case still excluded as VM evidence.

The actual production-wire integration passes against official Python RIC images on arm64 VZ and amd64/QEMU, computing42/7 with warm counters1/2, transferring near6MiB requests/responses and surfacing a real FunctionError. Each run independently removes both named containers, joins stream/wire owners and checks its run-label scope; subsequent daemon queries find no owned wire containers. These stress cases use60s IO/120s attempt budgets rather than claiming default5s performance. The separate five-case relay suite remains required regression evidence on each daemon.

W01's implemented filter now covers Hyper/hyper_util/Bollard independent of application TRACE. Production capture excludes request/response/header/partial-trailer markers while preserving application logs. The standalone compatibility suite passes eight tests/five doctests and Clippy, including actual Bollard create/logs construction/polls and truncated decoder fallback under LogTracer. It does not replace the stricter production framing decoder. Workspace/probe stacks align on Tokio1.54.1/Hyper1.12.0; root audit still has the same16 B01 advisories, deny bans/licenses/sources pass, and the standalone probe audit is clean. No ignore was added.

Deployment/image actors, remaining stream/network/probe consumers, warm pooling, startup/resource checks, full SDK/provider acceptance and B01/B03 remain required before P1 closes. W30 is closed for implemented wire admission/handoff; future log/pull owners remain subject to its existing-reservation rule. W31's source citations remain pinned and checked.

## P1 task1.3 local image resolution review

Independent design and implementation review found the issues below. All are fixed, with a final read-only recheck reporting no remaining concrete in-slice finding. The [source study](../docs/research/study-bollard-docker-transport-boundaries.md#platform-specific-image-identity-follow-up) pins the actual observations and historical Moby merge/store source.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W33 | P2; runtime §2.8 / `artifact/execution.rs:38` | A hash alone discards the defaults needed to preserve empty overrides without per-invoke inspection. ResolvedImage now retains validated Arc defaults and returns a fully materialized redacted Docker plan with the empty-entrypoint reset sentinel | Default/override/composed-budget tests, snapshot round trips and real create/inspect merge checks |
| W34 | P3; `docker/image.rs`, repository matching | Raw repository equality loses familiar Docker Hub digest evidence. Normalize default docker.io/library and index.docker.io, while classifying short-local provenance from original spelling | Familiar-name, registry-case, port/IPv6, unrelated-alias and short-local cases |
| W35 | P2; `artifact/image.rs:443` | Valid saved execution changes could retain a stale inspection hash; the selected descriptor digest was not retained. Metadata now stores selected_manifest and the constructor/deserializer recomputes centralized canonical SHA-256 over parent/child/platform/size/defaults | Ten valid-value mutation cases reject stale fingerprints; immutable defaults and Debug remain redacted |
| W36 | P2; `docker/image.rs:206` | Missing Descriptor alone can misclassify downlevel containerd as classic, including contradictory manifest summaries. Require matching classic inspect/info driver evidence, reject the containerd status marker, and reject explicit manifest summaries | API1.44/1.48 classic fallback, modern captured-parent selection, ambiguous summaries, malformed rows and btrfs/zfs marker negatives |
| W37 | P2; `docker/image.rs:108` | Requiring nonnull DriverStatus rejects legitimate classic VFS. Normalize absent/null rows after classic evidence, retaining marker/shape rejection | Missing/null VFS positives and containerd negatives; historical source pinned |
| W38 | P2; actual Docker qualification | Platform-qualified child Id is not independently addressable by lookup/create on the retained containerd store. Bind the requested tag once, platform-inspect its captured parent Id, retain child manifest separately and create from immutable parent plus explicit platform | Initial404 reproduced on both daemons; corrected two-case suites pass on each. Fake parent-a/child-b cases prove the second lookup cannot re-resolve a moved tag |

The resolver performs bounded read-only local inspection only. It validates selected Linux image metadata, inherited argv/environment/user/working-directory bounds, effective entry overrides and local/registry URI representation. No registry pull, credential discovery or image build occurs.61 new tests pass within288 Lambda core tests;58 core doctests pass. Workspace build/test/fmt/Clippy/rustdoc pass with1,964 tests and399 explicitly resource-gated cases. Native provider/DryRun regressions pass; the separately gated Squib case remains excluded as VM evidence.

Actual official-RIC wire execution now consumes resolver identities and materialized plans. Near6MiB computed warm calls/errors and a separate clear-command/reset-entrypoint create test pass on both arm64 VZ and amd64/QEMU, with independent stream/container cleanup. Default IO latency, old-daemon execution, native x86/Desktop/rootless and full provider/executor acceptance remain outside this evidence. Strict production Clippy has the same65 existing B03 findings, with none in the new resolver/defaults paths. Hex reuses an existing workspace version/package; audit still reports the16 B01 advisories and deny bans/licenses/sources pass without new ignores.

Deployment/status/CAS actors, image availability leases, allowed pulls, unpacked-image quota proof, environment/pool owners, remaining network/log/probe consumers and B01/B03 remain required before P1 closes. This slice does not advertise Docker invocation in Rustack.

## P1 task1.3 owned image preparation review

The independent read-only design and implementation review verified typed private-only mutation endpoints, shared lease lifetime, retained cancellation/cleanup, digest/platform unpacked proof, and worker/root recovery. The final recheck found no remaining concrete in-slice issue. Source review preserves the HTTP500 uncertainty rule: a returned nested containerd gRPC error does not prove its server stopped committing.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W39 | P2; runtime §2.9, private alias removal | Verify-ID and DELETE are separate operations; Docker has no expected-target CAS. Reserve the private namespace exclusively and state that concurrent external retargeting invalidates that guarantee. Observed mismatches quarantine without deletion | Wrong-target/repair regression; generated-only pin identities and force=false/noprune=true endpoints; pinned classic/containerd source |
| W40 | P3; `docker/lease.rs:65` | IO budget accepted subsecond positive durations despite the1..60s contract. Constructor now enforces the actual lower bound |1ns and over60s rejection plus typed config checks |
| W41 | P2; `docker/lease.rs:606`, self-review with independent recheck | Root failure with no registered rows could drop queued work without recording failure. Actor now records settled completion explicitly; abnormal Drop closes admission/marks failure even with zero rows | Empty-root failure, aborted-root cached failure, worker panic and off-runtime root Drop tests |
| W42 | P2; `docker/lease.rs:275`, self-review with independent recheck | Acquiring every semaphore permit to observe emptiness consumed other preparation capacity and distorted pin counts. Dedicated bounded count/watch observation now leaves admission capacity untouched | Wait-empty observer remains pending while a second legitimate preparation succeeds; clone lifetime and cancelled observer cases |

ImageLeaseManager now owns accepted preparations, live aliases and uncertain cleanup through retained workers/registry reservations. PinnedImage cannot be deserialized from metadata; clones keep the release channel alive, and shutdown revokes further use after consumers settle. Successful or discarded response delivery retains artifact ownership correctly. UnpackedImageSize validates1..10GiB; modern proof selects exactly one available image summary by digest/platform and reads Unpacked, with verified classic fallback. Preparation does not run user code or pull/build images.

Thirty-three new cases pass within321 Lambda core tests, including13 unpacked proof cases, two scalar/shared-lease cases and18 preparation/lifecycle cases. The latter cover exact precommit400/404 rejection, lost/postcommit/late nested500 outcomes, initial404 quarantine, no Tag replay, caller/response abandonment, bounded admission, alias conflict repair, lost/failed DELETE recovery, cancelled/timed shutdown, cached supervision failure and cleanup outside an entered runtime.76 core doctests pass. Workspace build/test/fmt/Clippy/rustdoc pass with1,997 tests and399 explicitly resource-gated cases. Native provider/DryRun regressions pass; the separately gated Squib case remains excluded as VM evidence.

The real Docker wire harness now acquires PinnedImages, keeps them through actual handler calls/container cleanup, and joins the image manager before DockerWire shutdown. Both two-case suites pass on arm64 VZ and amd64/QEMU, retaining near6MiB computed warm/error and empty-override behavior. Independent daemon queries find no private pin aliases. These are preparation/control-path qualifications, not provider/executor or default-latency acceptance. No dependencies/versions changed. Strict boundary checks report the same65 preexisting B03 findings, with no new pin/preparation/defaults-path finding; B01 remains an open dependency gate.

Next consume these artifacts in provider deployment/status/CAS and invocation capture, followed by pool/environment actors, startup directory ownership/previous-run reconciliation, allowed pulls, remaining stream/network/probe consumers and complete P1/M0 acceptance. No Docker execution capability is advertised by the current binary yet.

## P1 task1.3 deployment settlement and capture review

Independent read-only review covers typed deployment state, store-issued candidates, atomic settlement and fallible mutation rollback, snapshot migration, public status projection and leased invocation capture. It explicitly excludes the still-pending retained deployment supervisor and provider Image Create/Update admission/API wiring.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W43 | P2; `deployment.rs`, generated candidate builder | Public TypedBuilder generation bypassed store-issued candidate construction despite private fields. Builder method, type and final build are now module-private | Compile-fail external construction doctest plus independent recheck |
| W44 | P2; `function_config.rs`, resolved-image hash/size | Imported or mutated resolved records could claim an unrelated CodeSha256/CodeSize. Central canonical base64 fingerprint validation now covers store/import/capture; live sizes must match unpacked proof and restored sizes remain bounded | Tampered hash/zero/oversize snapshot rejection, live-size mismatch rollback and capture corruption tests |
| W45 | P3; `rustack-core/src/http.rs:375`, workspace validation | The existing2ms-progress/10ms-idle real-clock test can legitimately hit Idle when host scheduling stalls, defeating its total-deadline-only expectation. Pure timer tests now use Tokio paused time, with test-util explicitly enabled only for core dev builds | All five HTTP tests pass, including identical idle/tie/absolute-deadline assertions, without changing production behavior |
| W46 | P2; `provider.rs`, self-review of publication guard integration | Rejecting publication only inside PublishVersion would let Image Create(Publish=true) insert before returning an error. Create/Update now reject this unsupported asynchronous combination before code processing or mutation; initial candidates clear unverified hash/size | Create leaves no resource; update preserves old revision/hash/status; initial Pending omits CodeSha256 |

Twenty-five new Lambda core cases cover initial success/failure/retry, preserving old revisions during/after updates, independent resource/revision/generation CAS, two concurrent completion results, stale lease release, pending mutation/publication rejection, effective-entry rollback, published lease lifetime and resolved URI, snapshot state reconciliation and metadata-only availability. Invocation captures live leases through its captured request and rejects revoked/restored/tampered artifacts. The final independent recheck found no remaining in-slice issue. All25 new cases pass within346 Lambda core tests;92 core doctests pass, including the external builder rejection. Workspace build/test/fmt/Clippy/rustdoc pass with2,022 tests and399 explicitly resource-gated cases. Native provider/DryRun regressions pass; the separately gated Squib case is not VM execution evidence. Fresh target-only strict boundary validation reports the same65 preexisting Lambda findings and no finding in new deployment/artifact/fingerprint paths; the unfiltered attempt also exposes seven existing model documentation findings. These remain B03, not a green strict gate. B01 remains the16 recorded dependency advisories; no package/version/lock change or new ignore was introduced.

This foundation is not an accepted asynchronous deployment owner: it cannot prove bounded work-slot reservation, joined worker cancellation/panic or provider Pending admission until the next consumer lands. Docker capability remains unavailable; directory ownership/reconciliation, pulls, pool/environment ownership and P1 security/strict checks remain required. No new package/version is introduced; the core test-only Tokio feature reuses the existing version.

## P1 task1.3 retained deployment owner and API review

Independent read-only review covers accepted-work/mailbox reservation, atomic response receipts, registry/permit ownership above JoinSet children, store-notification cancellation, keyed settlement, root/emergency recovery and provider admission/status/revision behavior. Its final recheck finds no remaining in-slice issue. It does not close the production Docker pool/executor/probe gate.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W47 | P2; `deployment/manager.rs`, preparation select | Prioritizing global store notifications can starve a ready preparation. Poll preparation before unrelated notifications after deadline/cancel and retain post-result identity validation | Sixteen simultaneously-ready notification/result iterations plus one-future/no-replay actor coverage |
| W48 | P2; `docker/lease.rs`, overall/read deadlines | Accepted budgets up to3,600s were forwarded into the wire's600s limit. Validate total preparation separately and bound resolver, alias verification and unpacked proof by min(total, independent IO budget) | Successful actual wire/image preparation under a3,600s deployment budget |
| W49 | P2; self-review, abnormal root Drop | A worker can observe cancellation before abort/join recovery and incorrectly expose ordinary Cancelled for a failed owner. Project failed owner health to SupervisorFailed before settlement; preserve ordinary cancellation when health is sound | Off-runtime root Drop with in-flight tag, worker/root panic and cached-error regressions |
| W50 | P2; self-review, acceptance response capture | A metadata re-read after insertion can race deletion/recreation or fast completion. Store transactions now return an atomic acceptance receipt before enqueue, with no re-read | Delete between reservation and enqueue returns the original Pending receipt, settles stale work without poisoning admission and starts no tag |

The20 additional Lambda core cases pass within366 tests; the model adds one RevisionId serialization test. Workspace build/test/fmt/Clippy/rustdoc pass with2,043 tests, with400 resource-gated tests after adding the live SDK case.101 core doctests pass, including closed-candidate construction. Native provider/DryRun regression target passes six actual Native/validation scenarios; its separately gated Squib selection remains excluded as VM evidence. The target does not use an SDK/HTTP layer; previous labels in this spec set are corrected. The independent live Rust SDK revision test passes through the running Lambda HTTP gateway, verifying modeled/raw412 responses for all three operations, explicit nonempty advancing revisions and an immediate four-field nonmutation check before any subsequent successful update. Both two-case real Docker wire suites pass again on Engine29.5.2 arm64 VZ and amd64/QEMU after the read-budget change; subsequent alias/container ownership queries are empty. The harness retains60s stream IO/120s attempt budgets and does not establish full executor default latency. No Image Invoke or boto3 evidence is claimed by these tests.

Fresh target-only strict Clippy has the same65 preexisting B03 findings and none in new owner/lease/candidate code. Audit still reports16 B01 advisories; deny bans/licenses/sources pass while advisories fail, with no new ignore or dependency version/lock change. Next implement the Docker environment/pool executor, directory ownership/reconciliation, allowed pulls and startup probes/configuration before M0 can close; P2–P5 remain required.

## P1 B04 deployment validation review

The final independent read-only source review finds no remaining B04 issue, including the SDK rejected-creation absence check. Self-review corrected two admission-lifetime issues; the error-class correction was also rechecked against current source.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W51 | P2; self-review, `deployment_input.rs`, blocking-result ownership | Releasing capacity when CPU parsing ends allows ready large DTOs to accumulate. Return the permit with the result and retain it through provider dispatch | Completed-result capacity test plus cancelled waiter and worker panic capacity tests |
| W52 | P2; self-review, `provider.rs`, inline DryRun admission | Acquiring a second parse permit after HTTP decoding can reject all four already-admitted requests. Reuse the existing permit for the pure base64 worker | Four concurrent admitted validators succeed without nested admission |
| W53 | P2; review, `deployment_input.rs`, service-error conversion | Capacity and worker failure must not collapse into invalid-input400. Preserve TooManyRequests429 and Internal500 with static messages | Current conversion source independently rechecked; capacity/panic/invalid encoding cases remain separately classified |

29 additional core cases pass:395 tests,101 doctests. They cover ZIP/S3/image DryRun full-snapshot nonmutation, no staging/fetch/tag/publication, stale/qualified requests, strict nested schemas, large valid blobs, wrong-path metadata, exact base64 limits and parse/result ownership. Actual Rust SDK/HTTP validation passes (one explicitly selected case, zero ignored), including Create/UpdateCode/UpdateConfiguration unsupported fields, absent rejected resources, unchanged metadata and version count. The separate SDK revision/412 regression passes again. Native provider/DryRun regression passes six actual scenarios; the gated Squib selection is still not VM execution evidence.

Final workspace build/test/fmt/Clippy/rustdoc gates pass:2,072 tests, zero failed and401 resource-gated cases. No dependency package/version or Cargo.lock change landed; serde_json's existing raw_value feature supplies borrowed RawValue. Fresh strict core boundary Clippy still reports exactly65 B03 baseline findings, none in the decoder or newly extracted validation helpers. Fresh audit reports the same16 B01 advisories, and deny bans/licenses/sources pass while advisories fail; no new ignores. B04 is closed. B01/B03 and the remaining Docker executor/directory ownership/pulls/probes still block full P1/M0 acceptance; P2–P5 remain required.

## P1 B03 strict production boundary review

The six-library strict gate now covers rustack-auth, rustack-core, rustack-lambda-model, rustack-lambda-http, rustack-lambda-core and rustack-lambda-runtime. Fresh initial reports found65 core,47 authentication,7 model and4 HTTP findings (123 total); the shared core/runtime already passed. Method-specific Errors sections replace missing contracts, rustdoc tokens are formatted, and guarded credential/layer/date indexing uses exact slice patterns. Native output reads use checked access. These changes preserve the existing parser profile rather than claiming expanded layer compatibility.

The two production HMAC expect calls are removed by returning typed CryptoInitialization errors with their original InvalidLength sources. Public derive_signing_key/compute_signature now return Result, all production/header/presigned/streaming callers propagate it, and direct S3/SQS test helpers/examples are adapted. No infallible fallback, custom crypto/key normalization, hidden retry, new dependency/version or lint ignore was introduced. Two added tests check four independent published SHA1/SHA256 vectors, including a131-byte key; existing presigned ambiguity, payload-tamper, seed/chunk/trailer and constant-time verification regressions remain active.

| ID | Priority / source | Finding and resolution | Evidence |
|---|---|---|---|
| W54 | P3; `provider.rs`, delete_function Errors | Deletion can fail during root revalidation after metadata removal. Document both preflight and possible post-removal cleanup failure | Final source directly rechecked by independent reviewer |
| W55 | P3; `storage.rs`, validate_artifact Errors | Generic executable validation language overstates the check. Name bootstrap/component symlink checks and explicitly preserve missing-bootstrap acceptance | Final source directly rechecked; execution remains responsible for readiness |
| W56 | P3; authentication verifier Errors | Newly propagated MAC initialization failure was documented only on signing helpers. Add it to header/S3/presigned/streaming entry points | All public affected error contracts directly rechecked |

Self-review also corrected two provider descriptions: layer permission principals are not numerically validated here, and update event-invoke configuration creates absent records before merging. The final independent read-only review confirms all corrections and no remaining in-slice finding. Full P1/M0 acceptance remains separate.

Final-source workspace build/test/fmt/Clippy/rustdoc gates pass with2,074 tests, zero failed and401 resource-gated cases. `make test-lambda-boundaries` passes659 unit tests and165 doctests (824 total), seven separately gated Docker cases, and strict production Clippy for all six libraries without new ignores. Core remains395 tests/101 doctests; authentication is76 tests/12 doctests. The Native provider target again passes six actual Native/validation scenarios, excluding its gated Squib selection as VM evidence. The most recent audit/deny retains16 B01 advisories and passing bans/licenses/sources; this change has no manifest/version/lock change. Directory ownership/reconciliation, allowed pulls, real Docker pool/executor and startup/configuration/probes still block full P1/M0, and P2–P5 remain required.

## Existing baseline issues affecting final gates

| ID | Priority / source | Verified evidence | Required disposition |
|---|---|---|---|
| B01 | P1; `Cargo.lock:1462`, `:1474`, `:4308`, `:5893` | Baseline cargo audit finds 16 advisories across crypto-bigint 0.5.5/0.7.5, ctutils, quinn-proto, rustls/webpki roots and ZIP 8.6.0. cargo-deny fails its advisory gate; bans/licenses/sources pass. Standalone probe audit is clean | P1 dependency work: upgrade affected dependencies, including ZIP major and the SDK's SigV4a chain; rerun meaningful regressions and audit/deny. Do not add ignores to make the gate green |
| B02 | P2; `crates/rustack-kms-model/src/epoch_seconds.rs:39` | Existing optional epoch helper truncates negative fractional seconds and casts a negative fraction to u32; it is unsuitable as the generator's new codec | Do not reuse it in P2. Existing KMS correction is outside this workflow feature; retain as a repository follow-up without claiming it fixed |
| B03 | P2; Lambda model/HTTP/core and shared auth/core/runtime production libraries | **Closed:** the six-library strict target passes without new ignores after correcting123 fresh baseline findings, including typed MAC initialization errors, guarded indexing and method-specific Errors contracts | Retain `make test-lambda-boundaries` through P1 acceptance; this is not global workspace strict-lint or Docker executor acceptance |
| B04 | P2; `crates/rustack-lambda-model/src/input.rs`, `provider.rs` code-update dispatch | **Closed:** explicit DryRun, strict deployment DTOs and bounded decoder implemented; ZIP/S3/image nonmutation and live SDK rejection/absence/publication checks pass | Retain regressions through P1 acceptance; this fix does not establish Docker Image Invoke |

The P0 record distinguishes unchanged workspace advisory failures from the clean probe dependency graph; it never reports a fully green security baseline.

## Cross-references

- ← [implementation plan](./rustack-workflows-90-impl-plan.md)
- ↔ [runtime spike](../docs/research/spike-lambda-container-relay.md), [semantic oracle](../docs/research/spike-stepfunctions-semantic-oracle.md), [verification gates](./rustack-workflows-70-verification-plan.md)
