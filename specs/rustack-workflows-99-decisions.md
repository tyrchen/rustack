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

## D15 — Confirm poll consumption before committing runtime delivery

- Alternatives: treat a successful oneshot send as delivery; rely solely on biased select ordering across receipt and mailbox channels; leave an unconsumed event in Delivered until timeout.
- Decision: retain Invocation ownership in Offering until next() consumes its reply and acknowledges a per-poll receipt. Recover the queued event if that receipt closes. Reconcile ready receipts and revalidate stop/cancellation/deadline synchronously before each command/transition.
- Why: a sent response can be dropped without being consumed, and a receipt arriving between select branch polls can race a completion command. Explicit receipt ownership prevents lost events and false UnknownAttempt/initialization outcomes. Transport adapters still retire environments on failed HTTP delivery; session receipt is not proof of user-code execution.
- Pinned by: [shared session ownership and state diagram](./rustack-workflows-20-lambda-runtime-design.md#32-runtime-session-ownership).
- Date: 2026-10-25.

## D16 — Enforce HTTP dependency secrecy and retain adapter retirement

- Alternatives: redact only application Debug; trust RUST_LOG defaults; reject trailers after decoding without filtering the dependency; drop invalidated warm API handles without awaiting retirement.
- Decision: task-bound Hyper/hyper_util verbosity filtering independent of the application's subscriber, no supported HTTP trailers, and explicit joined API/process retirement on revision invalidation and shutdown. Cache supervision failures and retain the complete Native cleanup future across dropped/concurrent waiters.
- Why: pinned Hyper1.11.1 logs buffered trailer contents before application validation. Dropping a handle only signals asynchronous cleanup and cannot establish that an old listener's connections have finished. First-poll readiness, joined retirement and checked metadata let Native and the future Docker relay share one actual session engine.
- Pinned by: [shared HTTP adapter and Native startup](./rustack-workflows-20-lambda-runtime-design.md#33-shared-http-adapter-and-native-startup), [review W20–W22](./rustack-workflows-93-review.md#p1-task12-shared-http--native-migration-review).
- Date:2026-10-25. The separate mandatory Bollard filter W01 remains required when Docker is implemented.

## D17 — Retain relay IO ownership through bounded control settlement

- Alternatives: read stdin through Tokio's blocking adapter; read control frames inside a cancel-and-restart select branch; treat channel enqueue as successful emission; discard completed worker errors when priority stop wins.
- Decision: Linux nonblocking owned pipe descriptors, dedicated supervised reader/writer tasks, bounded two-message mailboxes and explicit flush receipts. Retain attempt tickets across cancelled result polls, join the API on every exit, and preserve completed IO failures while draining aborted workers. Matching duplicate Cancel is idempotent and never refreshes retirement.
- Why: a blocking stdin read can hold process shutdown open; restarting a partial read corrupts framing; an enqueued Ack does not prove it reached the host. Cancellation only invalidates the session, so the separate host owner must still terminate/remove the function. Real relay/RIC process tests complement the in-memory contract tests without claiming deployment/pool or SDK acceptance.
- Pinned by: [relay control supervisor and process entry point](./rustack-workflows-20-lambda-runtime-design.md#34-relay-control-supervisor-and-process-entry-point).
- Date:2026-10-25.

## D18 — Keep host protocol state separate from retained stream parsing

- Alternatives: let the Docker environment read/write raw frames directly inside cancellation selects; reconnect after attach failure; put container cleanup into the runtime library.
- Decision: an ordered host ControlClient consumes shared supervised stream workers. Mailbox receive is cancel-safe, partial parsing never restarts, and an interrupted send poisons later business operations until joined retirement. Close preserves ordinary validation of preceding events; cancellation discards only one matching late outcome before its acknowledgement.
- Why: Docker cancellation can race a flushed result or a partial read/write. The host must preserve framing and exact attempt identity without interpreting uncertain transport state as permission to replay or reuse. Container termination and allocation reservations remain with the environment owner that can prove them.
- Pinned by: [host control client and shared stream ownership](./rustack-workflows-20-lambda-runtime-design.md#35-host-control-client-and-shared-stream-ownership).
- Date:2026-10-25.

## D19 — Bound Docker transport before generated-model materialization

- Context: P1 consumer source review of pinned Bollard0.21.1 found whole-body collection, response-body work outside the header timeout, an attach decoder without a Rustack frame cap and console/trailing-EOF fallback.
- Alternatives: ordinary pipe client with post-deserialization checks; a counted custom connector plus a second raw attach transport; a local forwarding proxy; duplicate Docker model definitions.
- Decision: reuse model-only bollard-stubs1.53.1-rc.29.3.1 and implement the narrow, bounded Unix HTTP/upgrade boundary over the workspace Hyper/Tokio stack. Keep generated wire contracts, strict pre-materialization bounds, joined drivers and the mandatory dependency diagnostic filter.
- Why: checking after collection cannot bound earlier allocation, and an outer CBOR decoder cannot validate opaque Docker buffering first. The custom transport's Response<Incoming> type does not directly accept a Limited body wrapper. Owning the small required operation set avoids a proxy/second connector while preserving explicit cleanup and no-replay semantics.
- Supersedes: the initial recommendation to enable Bollard's ordinary pipe client in runtime §7; D2's relay architecture and the P0 feasibility evidence remain unchanged.
- Pinned by: [bounded Docker API boundary](./rustack-workflows-20-lambda-runtime-design.md#36-bounded-docker-api-boundary), [pinned source study](../docs/research/study-bollard-docker-transport-boundaries.md).
- Date:2026-10-25. Production acceptance still requires the specified fake-daemon and real image tests; source inspection alone closes no runtime gate.

## D20 — Separate lifecycle waits from destructive cleanup capacity

- Context: a Docker wait request can remain pending for an environment lifetime. Sharing two reserved management slots with waits can prevent Kill/Remove from ever reaching the daemon.
- Alternatives: use the ordinary business lane; increase one shared queue; poll container inspection repeatedly; allocate a dedicated bounded wait lane.
- Decision: retain six short business jobs and two reserved inspect/kill/remove/list jobs, plus a separate lane of at most32 concurrent wait jobs with an eight-command mailbox. Environment owners issue at most one wait per environment. Attach handoff releases its short management slot before warm reuse.
- Why: waiting for external termination must not consume the capacity needed to cause that termination. Separate admission and retained supervision provide bounded ownership without cleanup starvation.
- Pinned by: [Docker request ownership and quotas](./rustack-workflows-20-lambda-runtime-design.md#36-bounded-docker-api-boundary); executable held-wait and business-saturation regressions in the Docker wire tests.
- Date:2026-10-25. The wire lane does not itself own container lifetime or prove successful cleanup.

## D21 — Resolve the selected executable image, not its parent index

- Context: the live Engine29.5.2/containerd image store returns an index Id for unqualified inspection and a child-manifest Id for platform-qualified inspection; RepoDigests may continue naming the parent index, including for unpushed BuildKit-loaded local images.
- Alternatives: always use inspect Id without a platform; take the first RepoDigest; require a registry round trip for every local deployment; request/validate the selected platform and keep local/registry representation explicit.
- Decision: platform-qualified read-only local resolution validates executable child identity, retains validated inherited execution defaults and derives an explicit Docker-create plan preserving empty overrides, rejects unresolved index descriptors, and derives registry child references only from matching repository evidence. Capture the addressable parent Id before platform inspection, retain the selected child descriptor independently, and create by immutable parent Id plus explicit platform; the selected child Id is not necessarily a locally addressable image object. Recompute the canonical execution fingerprint during restore. Short local names retain local://Id. Legacy descriptor-free images remain supported on older compatible APIs.
- Why: deployment pinning must survive tag movement and daemon default-platform changes without inventing registry provenance. Inspection hashes must also be deterministic across irrelevant metadata/map ordering.
- Pinned by: [local resolution](./rustack-workflows-20-lambda-runtime-design.md#28-local-image-resolution-and-selected-platform-identity), [live/source evidence](../docs/research/study-bollard-docker-transport-boundaries.md#platform-specific-image-identity-follow-up).
- Date:2026-10-25. This refines D19's generated Id representation; it does not promise registry existence, cross-platform execution or completed deployment/provider integration.

## D22 — Own private image aliases through deployment and invocation lifetime

- Context: immutable IDs prevent tag drift but do not retain addressability when mutable image references are replaced. A prepared revision also needs unpacked-size proof rather than containerd compressed Size.
- Alternatives: rely on an operator tag/ID remaining present; hold never-started containers per revision; create private aliases under validated owner/run/pin identities with retained lease supervision.
- Decision: a bounded image lease actor prepares a private alias, verifies immutable identity and unpacked quota, and publishes a shared live artifact; only known private aliases can be removed. Record mutation phase before IO and retain uncertain cleanup/reservations across caller cancellation, deadlines and supervised recovery.
- Why: published versions and captured calls must own artifact availability. Observing one absent alias after an uncertain tag does not prove that a late tag cannot appear; visible matching commit followed by removal can establish cleanliness. A root/worker failure must preserve these identities rather than merely release permits.
- Pinned by: [owned image preparation](./rustack-workflows-20-lambda-runtime-design.md#29-owned-image-pins-and-deployment-preparation), [transport/source study](../docs/research/study-bollard-docker-transport-boundaries.md).
- Date:2026-10-25. Provider status/CAS, allowed pulls, directory ownership and previous-run reconciliation remain separate required consumers.

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
