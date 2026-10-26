# Workflows 70: Verification, Security and Performance Gates

Status: acceptance contract; executed foundation evidence is recorded in the implementation ledger/review; full M0–M4 acceptance remains open · Applies to M0–M4

## 1. Evidence and oracle policy

Use independent expected fixtures from official AWS API/language documentation and sanitized, provenance-recorded AWS observations where available. Each fixture records source/date, SDK/model version, supported profile and normalized nondeterministic fields. Do not calculate expected results with the same evaluator under test. Existing bootstrap fixtures and the [Cargo Lambda spike](../docs/research/spike-cargo-lambda-runtime-execution.md) are useful evidence, not proof of Docker or ASL execution.

Ordinary CI is offline after image provisioning and never contacts AWS. Optional AWS differential tests require an explicitly configured isolated test account and an intentional command; they are not run as part of this specification task. If no AWS account is available in P0, use official protocol fixtures and a documented evidence limitation; do not claim cloud differential parity. Any unresolved behavior that changes acceptance semantics blocks the corresponding release gate until a documented fixture or narrower profile resolves it.

## 2. Required test matrix

| ID | Scenario | Observable acceptance | Gate |
|---|---|---|---|
| L01 | Real image handler returns a computed value and invocation counter | Correct value and increasing counter; no echo path; warm reuse visible | M0 |
| L02 | Official Python/Node RIC image plus a custom Rust RIC image | At least one official managed-language image and custom RIC pass; both Python and Node pass by M4 | M0/M4 |
| L03 | ImageConfig overrides ENTRYPOINT/CMD/WORKDIR, including explicit empty arrays | Correct argv/cwd; absent differs from empty; no shell evaluation | M0 |
| L04 | Tag moves A→B, publish A, update latest B, alias movement | Published A stays A, latest uses B only after update, old in-flight revision completes A | M0 |
| L05 | Wrong OS/architecture, no RIC, init/error, early exit, OOM, malformed Runtime API | Distinct failures, no successful payload and no retained unhealthy environment | M0 |
| L06 | Two concurrent functions and a third forged completion | No cross-invocation response, stale/duplicate/foreign request rejected | M0 |
| L07 | Timeout/HTTP cancellation during allocating/init/busy/cleanup | Terminal failure once; function/relay removed or quarantined under retained ownership | M0 |
| L08 | Docker unavailable, interrupted pull, missing image, deployment saturation | Bounded failure; previous revision preserved; no Active placeholder | M0 |
| L09 | Reserved=0, global/per-function admission, memory/environment caps | Explicit throttling; permits not released before cleanup; no unbounded allocation | M0 |
| L10 | Container SDK calls Rustack S3/DDB | Real local result, no host credential inheritance; test enables those services explicitly | M4 |
| L11 | provided.al2023 Cargo Lambda ZIP in Docker | Same Linux bootstrap artifact executes on its native architecture | M4 |
| A01 | Create/update/delete/list/tag/describe through Rust SDK and boto3 | Correct JSON protocol, ARNs, timestamp types and errors | M1 |
| A02 | Concurrent Start with same name/input and mismatched input | One execution; same original response for RUNNING duplicate; correct collision error otherwise | M1 |
| A03 | Definition update while execution waits | Running execution uses old revision; new execution uses new revision | M1 |
| A04 | Pass/Task/Choice/Wait/Succeed/Fail | Correct transitions, outputs and terminal status | M1 |
| A05 | Full InputPath→Parameters→result→ResultSelector→ResultPath→OutputPath | Merge uses original raw input; missing/null/selection distinctions match fixtures | M1 |
| A06 | Path quoting/index/wildcard/slice, invalid grammar and duplicate .$ output keys | Correct bounded results or creation diagnostics | M1 |
| A07 | Every allowed intrinsic and nesting/operation/output boundary | Documented AWS-compatible fixture outputs; unsupported functions rejected | M1 |
| A08 | Choice short-circuit, missing/null/wrong-type, StringMatches and timestamps | Guarded missing path safe; no coercion/regex injection; NoChoiceMatched diagnosed | M1 |
| A09 | Retry ordered matchers, zero/default counts, backoff/jitter, Catch merge | Exact attempt count and bounded delay; no hidden adapter retries | M1 |
| A10 | Lambda timeout versus Task timeout versus execution timeout | Distinct errors/terminal states; execution timeout cannot be caught internally | M1 |
| A11 | States.Runtime and explicit DataLimitExceeded matching | Fatal runtime bypasses Catch; ALL does not catch data limit; explicit matching does | M1 |
| A12 | History pagination, reverseOrder, data omission, forged/expired tokens | Stable snapshot boundary and causal IDs; no cross-scope access | M1 |
| A13 | History/byte/transition budget exhaustion | Explicit failure with reserved terminal record; never successful truncated execution | M1 |
| I01 | Direct versus optimized Lambda with FunctionError/HTTP200 | Correct result envelope and Task failure mapping | M1 |
| I02 | Child finishes before subscription consumer runs | Parent always receives terminal result, no missed wakeup | M2 |
| I03 | startExecution versus .sync versus .sync:2 | Acceptance-only detached child; waiting owned child; string/JSON Output distinction | M2 |
| I04 | Parent Stop/timeout, child timeout/failure, simultaneous completion | Correct ownership/cancellation/status; one settlement | M2 |
| I05 | Nested parents saturate open/dispatch limits | Rejection or progress, no parent-holds-child-slot deadlock | M2 |
| P01 | Parallel completes out of order, Map completes out of order | Outputs preserve definition/item order | M2 |
| P02 | Map empty input, ItemProcessor/Iterator, MaxConcurrencyPath, nested Map | Correct empty/legacy/current behavior, bounded frames/concurrency | M2 |
| P03 | Branch failure plus state-level Retry/Catch | Siblings cancelled/joined; whole container state reruns, not failed branch alone | M2 |
| C01 | Callback before/after Lambda response, Lambda failure after early callback | Provisional result settles only after successful invoke; errors are not hidden | M3 |
| C02 | Heartbeat reset, overall timeout, retry and old token reuse | Task deadline unchanged; old token cannot complete new attempt | M3 |
| C03 | Wrong-scope/forged/expired/duplicate tokens | Defined API errors; no plaintext in tracing/history/archive | M3 |
| O01 | Graceful shutdown during Wait/Retry/Map/.sync/callback | Correct ABORTED cutoff, downstream cleanup, no false snapshot success | M4 |
| O02 | Restore terminal history/definitions/names; import RUNNING record | Atomic restore; RUNNING becomes interrupted, never replayed | M4 |
| O03 | Corrupt archive, changed tag, missing pinned image, owner mismatch | No partial restore or unsafe cleanup; no tag substitution | M4 |
| O04 | Two Rustack instances share daemon; one crashes | Restart removes only owned abandoned containers; other instance survives | M4 |
| O05 | Infinite ASL loop, log flood, large nested payload, oversized frame | Bounded memory/CPU/output; health and Stop remain responsive | M4 |

Every row needs positive and failure-path assertions where applicable. Tests involving Docker must assert container ownership/cleanup after completion and failure; checking only response JSON is insufficient.

## 3. Test layers and automation

1. Unit/parameterized tests: compiler, Path/intrinsic evaluator, retry matcher, data pipeline and protocol error mapping. Use descriptive `test_should_` names and rstest.
2. Property tests: no panic on bounded malformed input; valid graph transitions stay in scope; generation invalidation prevents double completion; result ordering independent of completion order; history causes refer only to earlier events.
3. Paused-time tests: deadlines, backoff/jitter ranges, token lifetimes and completion/cancel races using Tokio test clocks. Use real clock tests only for integration boundaries.
4. In-process integration: actual providers and binary adapters; controlled external ports only for forcing failures. Avoid replacing the entire engine with mocks.
5. SDK/HTTP integration: AWS SDK for Rust plus boto3, exact endpoint and test credentials; verify headers, field casing, JSON strings and errors.
6. Docker integration: native-architecture official/custom images, fault injection and ownership checks on all required platform lanes. Missing Docker may skip a local optional suite, but a required CI lane must fail if prerequisites are missing.
7. IaC smoke: basic Pulumi aws.sfn.StateMachine create/refresh/update/destroy with supported fields and a deployed image Lambda; versions/logging/encryption outside profile must reject explicitly.

Add future Makefile targets: `codegen-stepfunctions`, `test-stepfunctions`, `test-lambda-docker`, `smoke-workflows`, and `verify-workflows`. Add image provisioning/build targets instead of shell scripts. Pin fixture-image digests/content IDs and expected test manifest entries. `verify-workflows` runs the exact required suites and checks that none were silently skipped. Reuse the existing build/fmt/clippy/audit/deny targets.

For Rust-relevant implementation work run cargo build, cargo test, cargo +nightly fmt, and cargo clippy -- -D warnings, plus the repository's stricter pedantic/boundary lint selection and existing applicable targets. Run cargo audit and cargo deny check when dependencies land. Specification-only edits use text/link checks per AGENTS.md.

The shared runtime contract suite is available as `make test-lambda-runtime`; it runs the library tests/doctests and strict production boundary Clippy without lint ignores. Protocol property tests use [proptest1.12](https://docs.rs/proptest/1.12.0/proptest/) with512 cases per property, exercising bounded malformed bytes, mutations of valid envelopes and arbitrary binary payloads. Only its std feature is enabled; subprocess/fork machinery is unnecessary for these deterministic bounded parser checks. Runtime-session/relay tests extend this same target as their consumers land; a passing codec suite alone does not satisfy Docker acceptance.

`make test-json-boundaries` verifies the shared core JSON preflight and its strict production-boundary lints. Test escaped-key duplicates, per-object key scope, UTF-8 byte limits, encoded escape work, container depth, aggregate object/array members, empty collections, malformed syntax/numbers, typed schema redaction and configurable limits. Two512-case properties exercise arbitrary bounded malformed bytes and generated recursive valid values. This supports the Docker metadata/ASL parser boundaries; it does not establish their higher-level semantics or executor acceptance.

`make test-lambda-runtime-docker` builds the selected architecture's relay and pinned official Python RIC fixture images, then explicitly executes every resource-gated test in the shared runtime's Docker suite. It requires WORKFLOWS_DOCKER_SOCKET and a matching WORKFLOWS_ARCH; missing prerequisites fail rather than skip. The CLI is only the test transport. Assert computed42/7, warm counter1/2, request metadata, cgroup/user/rootfs restrictions, distinct FunctionError, running-handler cancellation, pre-function Close, EOF/truncated/partial frames, idle expiry, failed setup and expired helper reaping. The ordered production ControlClient also consumes real container pipes and drives computed warm calls followed by separate function-error/busy-cancellation scenarios; join its stream supervisor independently of container/CLI cleanup on all exits. Every case reconciles its preallocated owned names/run label and joins its CLI process. These are relay/client qualification checks, not provider/deployment/pool acceptance for L01–L09. Native x86, Docker Desktop and rootless qualification still require their M4 lanes.

`make test-lambda-docker-wire` builds those images and explicitly qualifies production DockerWire management/attach IO without using the CLI for execution. Assert near6MiB input/output fragmentation, real computed warm calls, handler failure, Close/Ack, joined control/bridge tasks and empty owned run labels after removal. The15 fake-daemon/framing tests additionally pin bounded malformed responses/headers/trailers, typed encoding/version negotiation, reserved cleanup during business saturation, Kill while two waits are blocked, dropped callers, cancelled shutdown waiters, cached worker panic, slow response-body expiry and poisoned partial reads. Captured TRACE tests cover production Hyper requests/responses/trailers; `make test-workflows-spikes` separately exercises actual pinned Bollard request/response/EOF decoder log sites, including synchronous stream construction.

Current wire qualification on2026-10-25 passes on Engine29.5.2 Linux arm64 in native VZ and Linux amd64 under QEMU. Both near6MiB cases pass with60s IO/120s attempt stress budgets. These do not establish the default5s IO latency budget, native x86 performance, Docker Desktop/rootless support or full Lambda provider/pool/deployment acceptance. Independent daemon queries find no remaining containers bearing the wire harness ownership label after the runs.

Local image resolution now extends `test-lambda-docker-wire` with a second real create/inspect case: clear inherited CMD and reset ENTRYPOINT without daemon re-inheritance. The execution case uses the captured addressable parent Id plus explicit platform, and preserves the selected child manifest independently.61 unit additions cover validated inherited defaults/plans, deterministic base fingerprints, ten valid-value stale-hash snapshot mutations, familiar registry identities, actual captured-parent query encoding and legacy API1.44/1.48 classic evidence. Null/missing VFS status remains valid; explicit containerd markers and contradictory manifest summaries reject. Historical engine code is pinned at vendors/moby v28.0.0; fake older-version cases are not old-daemon execution qualification. Current two-case suites pass on both retained daemons. Local resolution does not establish deployment status/CAS, availability leases, allowed registry pulls or unpacked-image quota enforcement.

Owned image preparation now extends the same real wire suite: acquire private aliases/live PinnedImages, prove selected unpacked size, retain leases through handler/container lifetime, remove containers, release/join pins, then stop DockerWire. Both retained architecture lanes pass and subsequent scoped image queries find no private aliases. The33 additional cases cover size proof, shared lease release, precommit rejection, complete outer500 with delayed nested commit, initial404 uncertainty, discarded successful responses, conflicting aliases, lost/failed DELETE, capacity observation and retained shutdown/worker/root recovery including off-runtime Drop. Source-pinned exceptions treat only exact fresh-alias400/404 and wire Busy as no-effect; HTTP500 remains uncertain. These gates do not establish provider publication/CAS, snapshot live-lease reacquisition, previous-run reconciliation or a production executor.

## 4. Security acceptance

| Boundary | Required proof |
|---|---|
| HTTP/ASL/archives | Byte/count/depth bounds before expansion; duplicate-key rejection; invalid values cannot reach unchecked engine indexing |
| Docker control | Local trusted socket only, no user-selected host mounts/socket/devices, no Runtime API port publish |
| Environment | Unprivileged identity, limits actually enforced, read-only code/rootfs, bounded tmpfs/logs |
| Invocation identity | One active attempt per environment; duplicate/stale/foreign completion cannot settle another call |
| Secrets | Redacted Debug/tracing, no host credentials, token hashes/tombstones, injected token redaction before history persistence |
| Cross-service | Scope validated at compiler and dynamic invocation; adapter cannot bypass Lambda admission |
| Shutdown | No forgotten resources; cleanup uncertainty blocks successful consistency snapshot |

Apply AGENTS.md boundary lints: unwrap_used, expect_used, indexing_slicing and panic. Dependency security audit is required evidence, not a claim derived from popularity. This local executor does not claim hostile multi-tenant containment or IAM policy parity.

## 5. Performance and resource acceptance

Use a recorded reference machine: native-architecture Linux, 4 dedicated CPUs, 8 GiB RAM and local Docker; also record Desktop results separately. Provision images before timing. These are provisional budgets to measure during implementation, not existing benchmarks:

| Measurement | Budget / method |
|---|---|
| Pure 50-state Pass/Choice execution, 1 KiB input | p95≤100 ms over 100 sequential executions, no Docker |
| Warm image Lambda adapter overhead | p95≤50 ms beyond handler time over 100 calls, concurrency 1 |
| Preloaded-image cold environment | p95≤5s over 20 environments; report relay and RIC init separately |
| Stop observability | Terminal/readiness response within cleanup budget; no health starvation under pure loops/log flood |
| Bounded storage |≤configured history/frames/containers/log limits; record allocator/RSS overhead separately rather than treating quotas as a total RSS claim |
| Steady-state cleanup | No growing container/task count after 100 invoke/timeout cycles |

Do not introduce a benchmark crate early. First measure these integration workloads and profile deviations; use current criterion only when a stable hot-path benchmark is justified. A missed provisional budget requires recorded analysis and an explicit spec amendment, not deleting the measurement.

## 6. Cross-references

- ← [all component contracts](./rustack-workflows-00-prd.md#7-reading-order-and-cross-references)
- → [roadmap exits](./rustack-workflows-80-roadmap.md), [implementation gates](./rustack-workflows-90-impl-plan.md)
- ↔ [operations](./rustack-workflows-60-operations-design.md), [decisions](./rustack-workflows-99-decisions.md)
