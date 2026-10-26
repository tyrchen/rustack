# Workflows 99: Key Decisions and Glossary

Status: accepted for specification; implementation requires P0 evidence · Date: 2026-10-25

Supersede decisions with a new ID rather than rewriting their rationale after implementation. Component documents own detailed behavior; this document owns the reasons.

## D1 — Own a bounded STANDARD/JSONPath interpreter

- Context: other projects need actual state transitions and fault behavior, not only API-shaped responses.
- Alternatives: wrap Step Functions Local/LocalStack; embed a general durable-workflow product; implement every ASL dialect immediately.
- Decision: implement typed ASL compilation and an actor-based interpreter, initially STANDARD and the documented JSONPath profile.
- Why: Rustack owns correctness, shared-service integration and lifecycle without external emulator dependencies. Explicit limits avoid accidental JSONata or EXPRESS claims.
- Pinned by: [PRD scope](./rustack-workflows-00-prd.md#4-release-scope), [compiler/profile](./rustack-workflows-30-asl-design.md#1-compile-before-execution).

## D2 — Keep Docker Runtime API inside each execution environment

- Alternatives: publish a wildcard host Runtime API; assume host.docker.internal can reach host loopback; forward invokes to RIE; rewrite user-image entrypoints with an injected launcher.
- Decision: use a small relay plus the unchanged function image sharing a network namespace; control traffic uses framed Docker attach.
- Why: host loopback is not container loopback; publishing Runtime API complicates isolation; RIE is unnecessary when Rustack owns the Runtime API. A relay preserves arbitrary valid RIC image entrypoints and works without host-address assumptions for control traffic.
- Cost/evidence: two containers per environment and a versioned relay artifact. Full composition is unproven until P0; this decision does not waive that gate.
- Pinned by: [Lambda architecture/transport](./rustack-workflows-20-lambda-runtime-design.md#1-ownership-and-execution-contract), [P0](./rustack-workflows-90-impl-plan.md#3-p0--evidence-and-risk-retirement).

## D3 — Pin content at deployment, not on each invocation

- Alternatives: pull by tag per invocation; freeze only published numeric versions; treat local content IDs as registry digests.
- Decision: latest and published revisions both capture resolved image content and full execution configuration; local content identity is explicitly distinguished from registry manifest identity.
- Why: tag movement and warm reuse must not change code silently; version tests need real immutable artifacts, including ImageConfig and architecture.
- Pinned by: [image resolution](./rustack-workflows-20-lambda-runtime-design.md#2-image-resolution-and-deployment-lifecycle), [snapshot image references](./rustack-workflows-60-operations-design.md#3-shutdown-cancellation-and-snapshots).

## D4 — Preserve one Lambda admission and execution path

- Alternatives: call Docker directly from ASL; use an HTTP AWS SDK client against Rustack itself.
- Decision: binary-owned LambdaTaskPort adapter calls the existing provider through a cancellable invocation-handle contract.
- Why: reserved concurrency, versions, backend failures and cleanup must match ordinary Invoke; an internal port avoids protocol overhead and recursive HTTP request-permit deadlock.
- Pinned by: [integration ports](./rustack-workflows-40-integrations-design.md#1-internal-ports-and-dependency-direction).

## D5 — Compile definitions and keep execution mutation in one owner

- Alternatives: interpret serde_json maps each step; share mutable execution state through Mutex/RwLock; spawn one task per Map item.
- Decision: immutable indexed IR plus one ExecutionActor with bounded frames, timers and external-call futures.
- Why: validation happens before side effects, generations settle races consistently, and fan-out cannot become an unbounded task tree.
- Pinned by: [ASL compiler/scheduler](./rustack-workflows-30-asl-design.md), [history](./rustack-workflows-10-api-design.md#6-history-and-pagination).

## D6 — Implement `.sync` only for a real supported job integration

- Alternatives: accept `.sync` on Lambda/arbitrary tasks; equate child start acceptance with child success; implement ECS-shaped responses without ECS execution.
- Decision: ordinary Lambda invocation awaits its result; `.sync` and `.sync:2` apply to nested Step Functions in this release.
- Why: production AWS ASL compatibility matters more than permissive local syntax. Nested child completion is a real job boundary; string-versus-JSON Output and ownership cancellation are observable contracts.
- Pinned by: [allowlist](./rustack-workflows-40-integrations-design.md#2-integration-allowlist), [nested execution](./rustack-workflows-40-integrations-design.md#4-child-workflows-and-sync).

## D7 — Expose uncertain side effects rather than replay automatically

- Alternatives: retry every Docker transport failure; claim exactly-once function execution; release permits when the caller stops waiting.
- Decision: no hidden business-call retries; explicit ASL Retry may repeat work; generation invalidation and supervised cleanup retain uncertain resources.
- Why: a broken connection cannot establish whether user code already changed another service. Resource ownership must outlive response futures.
- Pinned by: [Lambda cleanup](./rustack-workflows-20-lambda-runtime-design.md#4-environment-states-pooling-and-cancellation), [error mapping](./rustack-workflows-40-integrations-design.md#3-lambda-requestresponse).

## D8 — Preserve snapshot semantics without claiming durable workflow recovery

- Alternatives: add a new database/WAL immediately; serialize live handles/timers; replay all RUNNING tasks on restart.
- Decision: persist definitions, terminal histories and independent name reservations; quiesce/cancel active work before normal saves; never replay imported RUNNING calls.
- Why: existing snapshots are graceful-save archives. Exactly-once recovery requires a separate durable command/outcome protocol and external-side-effect strategy; serializing structs cannot provide it.
- Pinned by: [snapshot contract](./rustack-workflows-60-operations-design.md#3-shutdown-cancellation-and-snapshots).

## D9 — Support callbacks with scoped credentials and provisional completion

- Alternatives: accept only callbacks arriving after Lambda returns; let an early callback hide Lambda failure; persist raw task tokens in diagnostics.
- Decision: register tokens before dispatch, stage early callbacks, commit only after successful invocation, and redact injected tokens before history persistence.
- Why: real handlers can callback immediately; old attempts must not complete new work, and task tokens authorize state transitions.
- Pinned by: [callback protocol](./rustack-workflows-40-integrations-design.md#5-callback-and-heartbeat-m3).

## D10 — Make local operational limits visible

- Alternatives: inherit unlimited Docker resources; enable code execution by default; silently discard history or unsupported fields.
- Decision: explicit Docker opt-in, startup probes, bounded quotas, local-only transport and detailed capabilities; retain existing YAML settings.
- Why: honest local restrictions are testable. Docker permissions, tmpfs memory accounting, native-architecture coverage and non-IAM role metadata cannot be mistaken for AWS parity.
- Pinned by: [operations](./rustack-workflows-60-operations-design.md), [security/performance gates](./rustack-workflows-70-verification-plan.md).

## D11 — Apply P0 protocol evidence before implementing consumers

- Context: independent review and actual transport/SDK probes exposed gaps in the original draft.
- Decision: require BootstrapReady, default omitted Lambda architecture to x86_64, use callback TaskTimedOut for known closed tokens, generate numeric epoch-second HTTP timestamps, and decode both nested Input/Output for `.sync:2`. Optimized nested dates use epoch milliseconds. Use StdinOnce=true for the environment's single attach lifetime.
- Why: actual EOF tests distinguish detach from process stdin closure; ordinary SDK behavior and pinned AWS-validated snapshots establish observable wire distinctions. Current official errors take precedence over older snapshots.
- Pinned by: [runtime spike](../docs/research/spike-lambda-container-relay.md), [semantic evidence](../docs/research/spike-stepfunctions-semantic-oracle.md), [required review fixes](./rustack-workflows-93-review.md).
- Date:2026-10-25. These amendments refine D2/D6/D9 without changing the complete M0–M4 scope.

## D12 — Separate resource, resolved revision and invoked reference

- Alternatives: identify warm environments by function name/alias; use configuration RevisionId as the resource lifetime; reconstruct the invoked ARN from the resolved version.
- Decision: allocate a stable nonnil resource UUID, capture resolved execution configuration by value, and preserve the caller's qualifier separately for Runtime API metadata.
- Why: a recreated name must never acquire a deleted resource's environment, aliases can move without rewriting handler metadata, and snapshots must preserve the same resource identity. Importing legacy snapshots assigns an identity once; invalid or duplicate saved identities reject restore.
- Pinned by: [invocation capture](./rustack-workflows-20-lambda-runtime-design.md#24-invocation-capture-and-resource-identity).
- Date: 2026-10-25.

## D13 — Own code through staging, publication and invocation cleanup

- Alternatives: pass independent ZIP paths/bytes/image options; retain every rejected staging result until shutdown; drop extracted code with its function store.
- Decision: use a closed artifact enum, shared ZIP leases, validated immutable image metadata and a rollback guard for unpublished ZIPs. Validate normalized settings before source IO; commit new ownership only after insertion/CAS or full restore validation.
- Why: request cancellation must not delete live code or retain rejected uploads. The archive, extracted root and hash must describe the same staged data, and restoring a candidate must not leak its new artifacts on failure. Native process supervision retains its lease through child exit.
- Pinned by: [artifact representation and ownership sequence](./rustack-workflows-20-lambda-runtime-design.md#25-captured-artifact-representation).
- Date: 2026-10-25.

## D14 — Separate cancellation, caller response and resource settlement

- Alternatives: abort backend futures at the HTTP/Task deadline; release capacity as soon as a caller returns; rely on Squib's best-effort RAII shutdown.
- Decision: immutable Arc execution revisions, one attempt UUID/deadline, attached invocation handles and a bounded work actor. Backend cleanup survives dropped/expired waiters, and clean settlement follows resource cleanup and permit release. Idle VM Drop and Native warm-retention rejection follow the same ownership rule.
- Why: a caller timeout does not prove a process/VM has stopped. The pinned Squib0.2.0 startup/shutdown futures temporarily own VMM join handles; its raw Drop only attempts a shutdown message. Cancelling those futures cannot establish cleanup. Capture the parent Tokio handle for retained cleanup and shut down that runtime only after service settlement.
- Pinned by: [execution revision](./rustack-workflows-20-lambda-runtime-design.md#26-execution-revision-ownership), [cancellation and supervisor contract](./rustack-workflows-20-lambda-runtime-design.md#27-cancellable-invocation-and-supervisor-contract).
- Date: 2026-10-25.

## Glossary

| Term | Meaning in this specification |
|---|---|
| State machine revision | Immutable ASL/config snapshot captured when an execution starts; not a public Step Functions version/alias |
| Lambda execution revision | Resolved function code plus all execution configuration captured per invocation attempt |
| Execution | One STANDARD state-machine run, not one Lambda invocation |
| Frame | Engine-owned state/branch/iteration context inside an execution; not necessarily a Tokio task |
| State entry | One visit to a state; its retries share raw input and retrier counters |
| Attempt/generation | One dispatch and its validity epoch; stale results/timers cannot complete newer work |
| Execution environment | One function container plus its relay/netns; processes one invocation at a time and may be warm-reused |
| RIC / RIE | Runtime Interface Client inside the function versus Runtime Interface Emulator; Rustack needs RIC compatibility, not RIE forwarding |
| RequestResponse / `.sync` / callback | Await API/function response versus job termination versus token-authorized completion; distinct integration patterns |
| Owned / detached child | A `.sync` child is cancelled with its parent; start-only child remains independently supervised |
| Quarantine | A tracked environment whose cleanup is uncertain and still consumes allocation capacity |
| Snapshot / recovery log | A completed graceful-save archive versus a crash-consistent record of accepted commands/outcomes; only the former exists in this release |

## Cross-references

- ← [PRD](./rustack-workflows-00-prd.md)
- ↔ [roadmap](./rustack-workflows-80-roadmap.md), [implementation plan](./rustack-workflows-90-impl-plan.md), [survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md)
