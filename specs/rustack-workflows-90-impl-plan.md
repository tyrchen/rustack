# Workflows 90: Dependency-Ordered Implementation Plan

Status: P0 complete; P1 in progress; complete M0–M4 implementation objective remains active · Date: 2026-10-25

## 1. Readiness and dependencies

Available: Lambda provider/control plane, object-safe Executor, Native/Squib, immutable revision/artifact capture, invocation admission/cancellation, shared RuntimeSession/HTTP/relay/control client, bounded DockerWire, platform-aware local image resolution and owned image preparation/leases, existing gateway/YAML/runtime/snapshot infrastructure. Missing: provider asynchronous image publication/status/CAS, leased invocation capture, Docker environment/pool executor, startup ownership/reconciliation/probes, allowed pulls and final configuration/capabilities, Step Functions protocol/model/core, ASL engine, task integrations, workflow snapshots and complete acceptance fixtures. The execution ledger below distinguishes implemented foundations from full phase exits.

The [survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md) validates documentation, not a working relay. Existing [Cargo Lambda evidence](../docs/research/spike-cargo-lambda-runtime-execution.md) establishes the bootstrap protocol but cannot close Docker/platform risks. P0 must precede production implementation. P0 subsequently initialized the pinned vendor tree and inspected the specifically cited AWS-validated nested test/snapshot pairs; no complete upstream interpreter review is claimed.

```text
┌─ P0 evidence ──────────────────────────────────────────────────────┐
│ relay/RIC/platform spike │ protocol fixtures │ source/deps audit  │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ shared contracts, landed before consumers ────────────────────────┐
│ revision/artifact enum │ invocation handle/cancel │ scope/errors    │
│ RuntimeSession + transport │ supervisor ownership │ budget schema  │
└───────────────┬──────────────────────────────┬─────────────────────┘
                ▼                              ▼
┌─ P1 / M0 ────────────────────┐   ┌─ P2 foundations ────────────────┐
│ image resolver + Docker pool │   │ Smithy API + registry + compiler│
│ relay + provider lifecycle   │   │ actor/timer/history spine       │
└───────────────┬──────────────┘   └──────────────┬───────────────────┘
                └────────────────┬───────────────┘
                                 ▼ P2 closes M1: real Lambda tasks
┌─ P3 / M2 ──────────────────────────────────────────────────────────┐
│ frame scheduler → Parallel/Map → child start/subscription/.sync    │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼ P4 / M3: token registry + callbacks
┌─ P5 / M4 ──────────────────────────────────────────────────────────┐
│ ZIP containers │ snapshots/shutdown │ platform + fault + IaC gates │
└───────────────────────────────────────────────────────────────────┘
```

## 2. Effort model

| Phase | Outcome | Engineering days |
|---|---|---:|
| P0 | Resolve load-bearing risks |4–6|
| P1 | Close M0 |12–17|
| P2 | Close M1 |15–22|
| P3 | Close M2 |10–15|
| P4 | Close M3 |6–9|
| P5 | Close M4 |9–14|
| Total | Complete this specification set |56–83|

One developer plus 25% calendar overhead gives 14–21 weeks. Estimates explicitly include test/error/cleanup work; they are not benchmarks or guarantees. Independent fixture preparation can overlap design work, but API/adapter contracts and cancellation semantics must stabilize before consumers. Do not count theoretical parallelism as completed evidence.

## 3. P0 — Evidence and risk retirement

| Task | Deliverable | Contract | Days |
|---|---|---|---:|
| 0.1 | Reproducible isolated relay spike; Docker attach framing/EOF, netns sharing, official RIC, unprivileged limits and owned cleanup | [Lambda §§1–5](./rustack-workflows-20-lambda-runtime-design.md) |1–2|
| 0.2 | Semantic fixture catalog and memo: input equality, data null/missing, intrinsic/Choice edge cases, history event shapes, nested result/date types, boundary error matching | [ASL](./rustack-workflows-30-asl-design.md), [verification §1](./rustack-workflows-70-verification-plan.md#1-evidence-and-oracle-policy) |1–2|
| 0.3 | Pin official SFN model commit/hash and chosen SDK/fixture-image versions; prove numeric timestamp generator compatibility and dependency feature/security status | [API §1](./rustack-workflows-10-api-design.md#1-protocol-and-crate-boundaries) |1|
| 0.4 | Record measured spike outcomes, amend decisions/design/profile and recalibrate estimates; resolve incompatible assumptions | [decisions](./rustack-workflows-99-decisions.md) |1|

Artifacts: [relay spike](../docs/research/spike-lambda-container-relay.md) and [semantic oracle](../docs/research/spike-stepfunctions-semantic-oracle.md), with retained runnable probes, fixtures and exact commands/environment. Both memos now exist and are indexed. Spike-only automation belongs in Makefile if retained in the repository; no production workaround is hidden in the experiment.

**Exit:** actual relay+RIC execution and failure cleanup pass on at least Linux amd64 and macOS arm64; Linux arm64 lane is provisioned; source/fixture provenance is recorded; no unresolved question can change the selected public contract. Cloud differential evidence may remain unavailable only under the verification plan's explicit narrower-evidence policy. A failed spike requires a design amendment before P1.

## 4. P1 — Runtime contracts and real Image Lambda, closes M0

| Task | Strict dependency order | Contract | Days |
|---|---|---|---:|
| 1.1 | Land validated artifact/revision enum, full ImageConfig/ephemeral propagation and cancellable invocation handles; adapt HTTP and existing backends without behavior regressions | [Lambda §§1–2](./rustack-workflows-20-lambda-runtime-design.md), [ports](./rustack-workflows-40-integrations-design.md#1-internal-ports-and-dependency-direction) |3–4|
| 1.2 | Extract RuntimeSession state; add bounded relay codec/server artifact, socket client and startup probes | [Lambda §3](./rustack-workflows-20-lambda-runtime-design.md#3-runtime-api-and-relay-transport) |3–4|
| 1.3 | Add image resolution/digest records, Pending/update lifecycle, deployment workers, revision publication and pool/environment actors | [Lambda §§2–5](./rustack-workflows-20-lambda-runtime-design.md), [configuration](./rustack-workflows-60-operations-design.md) |3–4|
| 1.4 | Wire provider/config/capabilities, remove obsolete Docker rejection branches, implement cleanup/quarantine/reconciliation and L01–L09 | [verification §2](./rustack-workflows-70-verification-plan.md#2-required-test-matrix) |3–5|

Add appropriate Makefile fixture-image/runtime build and `test-lambda-docker` targets. Update workspace dependencies/features once, auditing bollard and the relay artifact. Remove obsolete fallback branches rather than deprecating unavailable execution paths. Keep default disabled, Auto and Native/Squib behavior explicit.

**Exit:** ordinary SDK invokes a real image on the P0 lanes, returns computed output, distinguishes failures, preserves published code across updates, and cleans/quarantines every failed allocation under bounded ownership. Existing Lambda quality gates and `make test-lambda-boundaries` pass, including [runtime §7.1](./rustack-workflows-20-lambda-runtime-design.md#71-strict-production-boundary-closure-b03) typed authentication error propagation and method-specific Errors docs. No requirement is satisfied using a noop/echo executor.

## 5. P2 — API, compiled semantics and sequential engine, closes M1

| Task | Strict dependency order | Contract | Days |
|---|---|---|---:|
| 2.1 | Implement protocol-aware numeric timestamp generation with required/optional/fractional/negative tests; import pinned model/config, generate model/HTTP, add service routing and feature/settings/health registration; registry/revisions/names/tags/read views | [API §§1–5](./rustack-workflows-10-api-design.md) |3–4|
| 2.2 | Build bounded strict compiler, scoped IR, Path/intrinsic evaluators and exact data transformations against fixtures | [ASL §§1–2](./rustack-workflows-30-asl-design.md) |4–6|
| 2.3 | Execution actor/timer/generation/history spine, six sequential states, Retry/Catch/deadlines and shutdown ownership | [ASL §§3–4](./rustack-workflows-30-asl-design.md), [API §6](./rustack-workflows-10-api-design.md#6-history-and-pagination) |4–6|
| 2.4 | Binary Lambda bridge, direct/optimized results/errors, Rust SDK+boto3 coverage and complete A01–A13/I01 | [integrations §3](./rustack-workflows-40-integrations-design.md#3-lambda-requestresponse) |4–6|

Add `codegen-stepfunctions`, `test-stepfunctions` and a sequential `smoke-workflows` target. Compiling a not-yet-implemented Parallel/Map/callback definition returns a precise unavailable-profile diagnostic until its milestone. Model presence alone does not advertise execution support.

**Exit:** PRD example executes its real image through ASL; failure, Retry/Catch, timeout and history semantics pass independent fixtures through both SDK clients. Duplicate Start/update races and forged pagination are tested. Stop cannot be starved by a tight loop.

## 6. P3 — Structured fan-out and nested jobs, closes M2

| Task | Dependency order | Contract | Days |
|---|---|---|---:|
| 3.1 | Extend frame scheduler for lazy branches/items, stable result ordering and global/per-execution frame budgets | [ASL §5](./rustack-workflows-30-asl-design.md#5-parallel-and-inline-map) |3–4|
| 3.2 | Parallel/Map Retry/Catch, legacy normalization, sibling cleanup and P01–P03 | [ASL §5](./rustack-workflows-30-asl-design.md#5-parallel-and-inline-map) |4–6|
| 3.3 | Child start/subscribe/cancel contracts, .sync/.sync:2 output mapping, admission/depth rules and I02–I05 | [integrations §4](./rustack-workflows-40-integrations-design.md#4-child-workflows-and-sync) |3–5|

**Exit:** all eight state types run; nested workflow example waits for actual child Lambda completion; string/JSON result distinction, fast-child races, cancellation, deadlock saturation and out-of-order fan-out tests pass. Unsupported service suffixes still reject.

## 7. P4 — Callback tasks, closes M3

| Task | Dependency order | Contract | Days |
|---|---|---|---:|
| 4.1 | Token hashes/scope/generation/tombstones, provisional callback protocol and injected-token history redaction | [integrations §5](./rustack-workflows-40-integrations-design.md#5-callback-and-heartbeat-m3) |2–3|
| 4.2 | Lambda callback adapter, SendTask* routing, heartbeat/Task timers and cancel/retry invalidation | [integrations §5](./rustack-workflows-40-integrations-design.md#5-callback-and-heartbeat-m3) |2–3|
| 4.3 | Actual callback-capable image fixture, C01–C03, token exposure scans and race coverage | [verification](./rustack-workflows-70-verification-plan.md) |2–3|

**Exit:** real handler callbacks work before/after Lambda return; Lambda failure cannot be hidden; old tokens never settle a new attempt; tracing/history persistence do not contain injected plaintext token values.

## 8. P5 — Persistence and consuming-project qualification, closes M4

| Task | Dependency order | Contract | Days |
|---|---|---|---:|
| 5.1 | provided.al2023 Docker ZIP execution and same-artifact fixture | [Lambda §6](./rustack-workflows-20-lambda-runtime-design.md#6-responses-errors-and-zip-support) |2–3|
| 5.2 | Workflow quiescence ordering, versioned service archives, image reference metadata and atomic restore validation | [operations §3](./rustack-workflows-60-operations-design.md#3-shutdown-cancellation-and-snapshots) |3–4|
| 5.3 | Three-platform lanes, server-in-container topology, fault/load cases, SDK service callbacks, IaC lifecycle and L10/L11/O01–O05 | [verification §§2–5](./rustack-workflows-70-verification-plan.md) |3–5|
| 5.4 | Final documentation/capability reconciliation, exact required-suite manifest and independent implementation review against this spec | [PRD goals](./rustack-workflows-00-prd.md#3-users-and-measurable-goals) |1–2|

Add `verify-workflows` and extend existing snapshot/smoke targets rather than introducing parallel scripts. Record measured performance and platform limitations. Update existing service documentation only with features actually verified.

**Exit:** every required verification row passes, no required lane silently skips Docker, all repository Rust/dependency checks relevant to the implementation pass, and unresolved correctness/security findings are fixed. Snapshot behavior and local compatibility limitations are user-visible. This closes the requested work.

## 9. Why this order is necessary

Cancellation/revision contracts precede both the Docker executor and workflow bridge; otherwise stopping a Task leaks resources or reuses mutable code. Compiler and history semantics precede Parallel/Map so branch machinery has one data/error contract. Child subscription/admission rules precede `.sync` so fast completion cannot be missed and parents cannot block child capacity. Snapshot integration follows fully settled lifecycle semantics, avoiding archives that appear resumable while silently replaying side effects.

Implement phases with the repository's impl workflow and its independent review requirement. Specification publication and P0 feasibility evidence alone do not establish production runtime acceptance.

## 10. Cross-references

- ← [survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md), [PRD](./rustack-workflows-00-prd.md)
- ↔ [roadmap](./rustack-workflows-80-roadmap.md), [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)

## 11. Implementation execution record

The complete implementation objective includes P0–P5. This ledger records evidence, not reduced scope. TaskCreate is unavailable in this environment; task rows below provide the phase tracking required by the impl workflow.

| Task | State | Evidence / next action |
|---|---|---|
| 0.1 | Evidence complete | Real official RIC image execution, warm reuse, handler failures, cgroup/nonroot/rootfs checks, busy-handler cancellation, attach EOF comparison and label reconciliation passed on arm64 VZ and Linux amd64/QEMU. Native x86/Desktop/rootless qualification remains M4. |
| 0.2 | Evidence complete | 48 independent semantic/protocol expectations; four offline SDK serializer/parser checks; pinned AWS-validated nested shapes and error corrections. No interpreter/cloud-differential execution is claimed. |
| 0.3 | Evidence complete | Model commit/hash and 37 operations pinned; six generated files compile in the standalone experiment. Numeric timestamp failure independently reproduced and mandatory generator remediation assigned task 2.1. Rust updated to current stable 1.99.1. |
| 0.4 | Complete | Independent final recheck confirms W07–W09 fixed, zero owned containers on both daemons, and no further P0 correctness finding. W01–W06 contracts are corrected; production consumers remain in their assigned phases. Full workspace build/test/fmt/Clippy/rustdoc pass; unchanged baseline audit/deny advisory failures are B01 and must be fixed in P1 dependency work. |
| 1.1 | Foundation implemented and reviewed; P1 gates remain open | Resource/revision capture, closed artifact/architecture types, shared ZIP ownership, validated immutable image metadata, early settings preflight and transactional staging rollback are implemented. Arc ExecutionRevision and cooperative InvocationHandle/WorkSupervisor now preserve attempts, deadlines, cleanup ownership and bounded admission across Native/Squib. Independent W13–W15 findings fixed/rechecked. Workspace:1,767 passing tests; Lambda:215 unit tests, five real Native fixture tests and native provider/DryRun regressions pass. Actual Squib VM regression is not claimed. Baseline B01/B03 are now closed by the security and six-library strict passes in §§7.1–7.2; remaining P1 runtime acceptance still applies. |
| 1.2 | Runtime contracts implemented/reviewed; Docker consumer/probe gates remain | Protocol4da7df3, shared actor2c50c2d, Native migration95282c4 and relay6779f93 are committed. The ordered host ControlClient now shares retained stream workers with the relay, validates causal attempt/cancel/close messages and poisons uncertain sends. Workspace:1,867 tests; runtime:97 tests/47 doctests and strict boundary Clippy; actual Linux entry-point Clippy passes. Five explicit Docker qualification tests pass on each arm64 VZ and amd64/QEMU daemon, including production ControlClient→real relay→official RIC computed warm calls, FunctionError, busy cancellation and independent stream/container cleanup. W27–W29 are fixed/rechecked. Startup deployment/resource probes still require the task1.3–1.4 Docker consumer; no production executor/provider acceptance yet. |
| 1.3 | In progress | Pinned transport study/§3.6/D19 define generated-type reuse, bounded Unix HTTP/upgrade IO, retained handoff and reserved cleanup slots. Shared core JSON preflight is implemented/reviewed before the Docker model consumer:21 new parser tests, two512-case properties, four core doctests and strict core boundary Clippy pass. Workspace:1,888 tests; audit remains unchanged B01. DockerWire now implements typed management and strict attach with15 fake-daemon/framing tests, retained IO/driver supervision and separate lifecycle-wait capacity (W32/D20). Real near6MiB official RIC wire tests pass on arm64 VZ and amd64/QEMU; independent review finds no remaining in-slice issue. Workspace1,903 tests and standard gates pass. Read-only ImageResolver and retained validated defaults/plans now bind platform inspection to captured addressable parent IDs, preserve selected manifests, verify snapshot fingerprints and support explicit platform create.61 additional tests and corrected two-case real Docker suites pass; independent W33–W38 fixes are rechecked. Workspace1,964 tests and standard gates pass. Owned ImageLeaseManager/PinnedImage preparation now preserves private-alias availability, proves unpacked quota, retains uncertain mutations and joins phase-aware cleanup/recovery.33 additional cases pass; real two-case suites on both retained daemons consume leases through actual container execution/cleanup. Independent W39–W42 fixes and final recheck pass. Workspace1,997 tests and standard gates pass. Typed deployment state/store settlement and leased invocation capture are implemented and independently reviewed (§2.10); initial/update failures, stale resource/revision/generation results, rollback and metadata-only restore are covered by25 additional cases (346 core tests,92 doctests). Workspace2,022 tests and standard build/test/fmt/Clippy/rustdoc gates pass; Native provider/DryRun regressions pass without claiming the gated Squib case as VM evidence. W43–W46 fixes are reviewed; strict B03 and dependency B01 remain open. Retained ImageDeploymentManager and provider Image admission/status/revision wiring are implemented/reviewed (§2.11).20 core additions and one model wire-field case pass (366 core tests,101 doctests; workspace2,043 tests). Actual Rust SDK/HTTP revision/412 regression passes separately from Native provider execution tests. W47–W50 fixes pass. B04 is now closed by explicit side-effect-free code-update DryRun, strict deployment DTOs and bounded borrowed-blob JSON decoding (§2.12).29 additional core cases pass (395 core tests,101 doctests; workspace2,072 tests,401 gated cases), and actual SDK/HTTP validation and revision tests pass. W51–W53 fixes/recheck pass. B03 strict closure is independently reviewed (§7.1), with all six production libraries passing without new ignores; two added HMAC-vector tests preserve signing semantics. Final workspace2,074 tests and standard gates pass; the boundary target passes659 unit tests/165 doctests and strict Clippy for all six libraries. B01 dependency closure is implemented/reviewed (§7.2/D27, W57–W59): nine ZIP regressions, workspace2,083 tests,833 boundary tests/doctests, green standard/strict gates, clean audit and all deny checks. Regional SDK/HTTP and Native provider regressions pass. Production Docker execution is still disabled. Directory ownership/previous-run reconciliation, allowed pulls, remaining stream/network/probe consumers and environment/pool integration remain required. |
| 1.4, P2–P5 | Pending | Remain required; no production Docker executor/provider or workflow-engine acceptance is claimed yet. |

P0 measurements validate architecture feasibility, not production throughput or reduced delivery estimates. Keep the existing P1–P5 estimates until an implemented executor/engine provides calibration. B04 side-effect-free code-update DryRun and bounded request validation are implemented and independently reviewed with live SDK evidence. Next implement task1.3 Docker environment/pool actors and the real executor consuming the retained deployment manager, typed settlement, ImageLeaseManager and qualified wire/client/relay/HTTP/session contracts. Complete directory ownership/previous-run reconciliation, allowed pulls and remaining stream/network/probe consumers, followed by task1.4 startup/configuration and acceptance. B01/B03/B04 are now closed with the security, six-library strict boundary and live SDK/API evidence in §§7.1–7.2/D27. These baseline closures do not enable Docker execution or close M0; runtime consumers/probes and P2–P5 remain required. Docker execution is not yet advertised as available in Rustack.
