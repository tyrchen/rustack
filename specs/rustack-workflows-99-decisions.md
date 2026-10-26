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

## D23 — Store-issued deployment generations and live captured images

- **Context:** image preparation completes asynchronously while the old function may still execute, be updated, deleted or recreated. Snapshot metadata does not retain daemon availability.
- **Alternatives considered:** independently mutable status strings allow contradictory readiness/progress; revision-only settlement cannot distinguish repeated failed attempts with the same old code; deserializing an executable image from metadata invents ownership.
- **Decision:** use one typed DeploymentState, a module-private builder for store-issued ImageDeployment candidates, and resource UUID/original revision/generation checks in one commit transaction. ImageArtifact distinguishes unresolved/restored/live states; invocation and publication require a live PinnedImage.
- **Why:** failed updates preserve old executable code, stale/duplicate results cannot publish, and a snapshot cannot authorize an unowned Docker execution. One canonical base64 image fingerprint and proven unpacked size bind API metadata to captured content. A fallible store mutation commits only after its closure and record validation both succeed.
- **Migration:** snapshots retain a derived legacy state plus optional typed deployment; contradictory states reject, and abandoned in-progress work restores as a closed failure. Metadata requires exact-content availability reacquisition in P5.
- **Pinned by:** [runtime §2.10](./rustack-workflows-20-lambda-runtime-design.md#210-deployment-state-and-atomic-settlement), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-25.

## D24 — Retained deployment admission with atomic response receipts

- **Context:** provider requests may disappear after metadata mutation; deletion, shutdown and actor failure must not orphan Pending records or release capacity before worker join.
- **Alternatives considered:** spawning from the request loses joins on cancellation; reserving only a mailbox does not bound accepted workers; re-reading metadata after enqueue can return a concurrently recreated resource or a falsely Active acceptance response; forwarding the total deployment budget to every RPC violates the wire's shorter IO limits.
- **Decision:** reserve both accepted-work and owned-mailbox capacity before mutation, capture the response receipt in the same store transaction, register guards/permits above JoinSet children, and retain root/emergency joins through waiter cancellation. Store-change notifications cancel stale candidates without replaying preparation; ready results outrank unrelated notifications after deadline/cancel. Each Docker read is bounded independently of the total deployment budget.
- **Why:** request lifetime cannot own accepted asynchronous work. Stale results remain harmless, failed updates preserve old code, and failure/Drop outside the entered runtime still settles keyed records. Executor image readiness defaults to unavailable, so qualified preparation alone cannot enable a nonexistent production backend.
- **Compatibility:** explicit stale RevisionId/CodeSha256 preconditions return412; in-progress operations and implicit staging conflicts remain409. The Native fixture target bypasses SDK/HTTP and is provider execution evidence; separate live SDK wire tests establish protocol evidence.
- **Pinned by:** [runtime §2.11](./rustack-workflows-20-lambda-runtime-design.md#211-retained-deployment-supervisor-and-provider-admission), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-25.

## D25 — Explicit DryRun and bounded deployment parsing ownership

- **Context:** ignored DryRun can mutate code; blindly applying ordinary JSON string limits rejects legitimate inline ZIPs, while parsing the entire code envelope into generic values permits large metadata allocations.
- **Alternatives considered:** permissive DTOs silently discard unsupported modes; decoding ZIPs for validation adds staging and content inspection; releasing a parse permit at CPU completion permits large ready-result backlogs; acquiring another permit inside an admitted DryRun can throttle every admitted request.
- **Decision:** model code DryRun explicitly and validate syntax/preconditions without fetching, staging, Docker admission or publication. Reject unknown deployment fields. Borrow the sole allowed ZIP raw span, cap/validate all remaining metadata, and deserialize strict DTOs on at most four pure blocking workers. Carry permits with results through dispatch and reuse the admitted permit for inline validation.
- **Why:** validation must not produce deployment effects, request cancellation must retain bounded CPU/memory ownership, and supported inline package sizes must not weaken metadata limits. Preserve overload429 and worker-failure500 separately from invalid-input400. This is a deployment boundary change, not a claim of a global parser rewrite or IAM parity.
- **Pinned by:** [runtime §2.12](./rustack-workflows-20-lambda-runtime-design.md#212-side-effect-free-code-validation-and-bounded-deployment-json), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-25.

## D26 — Propagate cryptographic initialization errors through signing APIs

- **Context:** strict production checks expose two HMAC expect calls on a formally fallible provider API; authentication is part of the Lambda gateway dependency boundary.
- **Alternatives considered:** retaining panic/adding lint ignores violates the boundary contract; fabricating a signature or interpreting an initialization failure as a mismatch hides infrastructure failure; hand-rolling key normalization solely to obtain an infallible constructor duplicates cryptographic logic.
- **Decision:** keep RustCrypto HMAC0.13/digest0.11 computations, return typed CryptoInitialization with its InvalidLength source, and make public derive_signing_key/compute_signature return Result. Update callers/examples directly; provide no deprecated infallible shim. Header/presigned/streaming verifiers propagate failure before comparison/fallback or chain advancement.
- **Why:** no request/key can reach an explicit panic, source errors remain available without embedding credentials, and successful signatures retain their published-vector semantics. Existing S3-only normalized-fallback eligibility and constant-time comparison remain intact.
- **Pinned by:** [runtime §7.1](./rustack-workflows-20-lambda-runtime-design.md#71-strict-production-boundary-closure-b03), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-25. This is an internal-library API migration for direct consumers, not a new AWS wire mode or a dependency upgrade.

## D27 — Align SDK signing features with the regional local compatibility profile

- **Context:** B01 includes crypto-bigint0.5 through optional AWS SDK SigV4a; the latest released signer inspected (1.6.3) still directly requires0.5, while the patched bigint floor is0.7.6.
- **Alternatives considered:** an advisory ignore leaves the security gate false; an unreviewed crypto fork adds a maintenance/supply-chain burden; removing supported regional SigV4 clients would break the workflow contract.
- **Decision:** remove the unused explicit S3/EventBridge/STS SDK SigV4a features, retain their regional SigV4/HTTP/Tokio behavior and current model versions, and upgrade affected compatible libraries plus ZIP9.1. This selects the actual local client profile already enforced by the gateway; user-image SDKs remain independent.
- **Why:** the existing gateway fails closed for unimplemented ECDSA signing, and workflow acceptance has no multi-region AWS signing requirement. Security closure must prove the resolved graph, not merely rename or ignore a vulnerable version. Future multi-region support requires a repaired upstream signer and its own tests.
- **Pinned by:** [runtime §7.2](./rustack-workflows-20-lambda-runtime-design.md#72-dependency-security-closure-b01), [HTTP authentication profile](./rustack-http-hardening-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-25. SDK model versions and supported AWS wire modes are preserved.

## D28 — Retain an OS directory lease in image namespace capabilities

- **Context:** owner/run UUIDs alone cannot exclude another process, and a coordinator-only lock can close while retained Docker cleanup still runs.
- **Alternatives considered:** PID files/in-process maps do not establish OS exclusion; replacing a lock file splits its inode ownership; arbitrary caller-supplied UUID scopes lose the filesystem proof; following path-based symlinks can redirect managed writes.
- **Decision:** acquire a permanent advisory-locked descriptor in a private UID-checked directory, durably publish a bounded immutable owner record, and clone the lease through the scope/pin/job/recovery lifetime. Use safe nix0.31.3 relative-descriptor operations and standard File locking on bounded blocking workers; do not expose arbitrary production scope constructors. The [current nix openat API](https://docs.rs/nix/0.31.3/nix/fcntl/fn.openat.html) returns an OwnedFd with fs enabled; [standard File locking](https://doc.rust-lang.org/stable/std/fs/struct.File.html#method.try_lock) supplies nonblocking advisory exclusion. Verified 2026-10-26; no new package version is required.
- **Why:** process exclusion must last through cleanup settlement, persisted corruption must fail closed, and a failed/cancelled initialization cannot authorize Docker effects. Retaining a standard File is a deliberate lint exception because Tokio has no advisory-lock API; synchronous IO is confined to blocking workers. This capability does not prove previous-run reconciliation or backend readiness.
- **Pinned by:** [runtime §4.1](./rustack-workflows-20-lambda-runtime-design.md#41-persistent-directory-lease-and-namespace-capability), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-26.

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

## D29 — Retain function output framing independently of snapshot traffic

- **Context:** real Docker handlers can produce binary/flooding stdout/stderr while the control channel carries invocation results. Log snapshots must not cancel a partially consumed outer Docker record or retain unbounded output.
- **Alternatives considered:** ordinary buffered Docker logs can accumulate response bodies; sharing control transport introduces output backpressure into invocation delivery; cancelling next() for every snapshot poisons its partial parser; blocking consumers on a full ring can block the handler.
- **Decision:** attach live output only before function start using the existing joined upgrade transport; one retained actor owns framing and bounded channel/tail rings, preserving its pinned read future across read-only snapshot requests and prioritizing stop. Close failed/EOF sockets immediately while retaining final snapshots and join failures.
- **Why:** snapshot and log flooding have bounded service quanta, user bytes never enter infrastructure diagnostics, and environment cleanup can join this socket independently of function/relay termination. The capture alone does not prove container exit or exact per-invocation log attribution.
- **Pinned by:** [runtime §3.7](./rustack-workflows-20-lambda-runtime-design.md#37-retained-function-log-capture), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-26. No new dependency or version is introduced.

## D30 — Materialize one immutable, bounded environment allocation plan

- **Context:** container creation must bind captured function configuration to immutable image leases, namespace ownership and operator limits before any daemon side effect. Hand-built relay/function requests otherwise drift in resource, metadata and entry semantics.
- **Alternatives considered:** per-call tag resolution breaks captured versions; assembling separate arbitrary Docker bodies permits policy drift; concatenating Env tiers leaves duplicate names with undefined process lookup precedence; inheriting daemon logging can spill unbounded output outside Rustack rings.
- **Decision:** EnvironmentPlan owns the captured revision/leases and preallocated identities, validates policy and serializes fixed relay/function bodies; only typed create methods issue them. Compose exactly one environment entry per key with inherited→captured→authoritative precedence and explicit deletion markers. Disable daemon logging, healthchecks, restart and extra swap while retaining independent live attach capture.
- **Why:** one plan supplies resource admission totals and immutable allocation inputs for the future environment/pool owner. Docker create futures must be Send for supervised Tokio workers, and raw request serializers must leave scope before awaits. A plan neither reserves capacity nor proves startup/reconciliation readiness.
- **Pinned by:** [runtime §4.2](./rustack-workflows-20-lambda-runtime-design.md#42-immutable-environment-allocation-plan), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date:** 2026-10-26. No dependency/version change is required. Network none is deliberate pure-handler opt-in; bridge and resource probes remain mandatory for production enablement.

## D31 — Keep environment ownership above workers and track possible wire issuance

- **Context:** cancellation, task panic or a failed HTTP reply can outlive the allocation request. Releasing a count/memory/image/directory reservation before actual cleanup permits orphaned or unaccounted execution.
- **Alternatives considered:** worker-local quota guards disappear on panic; immediate404 cannot settle an in-flight/uncertain create; classifying every Stopped/Deadline as unissued can ignore a sent request; treating all protocol errors as uncertain supervision leaks otherwise-settled quota.
- **Decision:** a root actor retains plans, mutation phases, reservations and IO cleanup receipts above its JoinSet. A single RAII wire mutation guard moves with the accepted request, marks possible issuance before send and proves never-issued only after final disposal. Worker guards transfer control/log/bridge ownership to retained cleanup on Drop; root recovery joins workers/IO and reconciles verified IDs. Preserve Worker failure through recovery, independently of physical quota release.
- **Why:** every accepted effect retains an owner, while validation/connect/queue failures can settle without permanent false uncertainty. Health checks reschedule after completion to avoid backlog starvation; log EOF permits an already-observed valid result but forbids reuse. Known protocol failures remain business failures once IO joins; uncertain supervision cannot be masked by an earlier protocol error.
- **Pinned by:** [runtime §4.3](./rustack-workflows-20-lambda-runtime-design.md#43-retained-environment-owner-and-allocation-reservations), [verification](./rustack-workflows-70-verification-plan.md), [implementation](./rustack-workflows-90-impl-plan.md).
- **Date:** 2026-10-26. This low-level owner does not replace provider admission, warm/LRU pooling, startup reconciliation/probes or exact Lambda timeout/log integration. Production Docker stays disabled until those M0 consumers pass.

## D32 — Pool dispatch retains receipt-based ownership through warm return

- **Context**: §4.4 warm execution must not reuse an environment after a caller abandons a ready result or a dispatch worker fails.
- **Alternatives considered**: transferring handles to HTTP futures loses cleanup ownership; returning to the LRU immediately after send loses result-consumption cancellation; keying by redacted Debug hides configuration differences.
- **Decision**: retain pool tasks and environment retirement receipts above the JoinSet, require a consumption receipt before warm return and hash canonical typed allocation inputs.
- **Why**: queue, execution, warm retention and cleanup have explicit bounded owners; underlying EnvironmentManager reservations remain authoritative through quarantine.
- **Pinned by**: [runtime §4.4](./rustack-workflows-20-lambda-runtime-design.md#44-warm-checkout-and-retained-pool-dispatch), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26.

## D33 — Arm handler timeout at consumed Runtime API delivery

- **Context**: A warm RIC may not poll immediately; queue/startup time must not consume the function handler timeout or extend the caller's absolute deadline.
- **Alternatives considered**: starting at host dispatch charges queue/transport time prematurely; starting on offer charges abandoned polls; retaining protocol v1 permits an older relay to silently omit timeout enforcement.
- **Decision**: version2 carries a validated relative timeout; the session's consumed poll receipt arms the effective monotonic deadline and a distinct correlated timeout result forces environment retirement.
- **Why**: HTTP metadata and actor enforcement share one delivery instant, abandoned offers remain recoverable, and handler versus outer timeout preserves error semantics without replay.
- **Pinned by**: [runtime §3.8](./rustack-workflows-20-lambda-runtime-design.md#38-delivery-based-handler-clock-and-protocol-v2), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26.

## D34 — Separate invocation log windows from environment diagnostics

- **Context**: Warm reuse must not return already collected logs from an earlier call; independent stdout/control streams lack a causal flush barrier.
- **Alternatives considered**: returning the global tail leaks prior collected bytes; clearing diagnostic rings loses health evidence; inserting sleeps or rewriting images cannot establish a reliable producer-side boundary.
- **Decision**: use acknowledged Begin/Finish actor commands and an opaque per-attempt capability around one bounded reusable4KiB window, alongside existing diagnostic rings.
- **Why**: current-call observed output has bounded explicit ownership, silent calls cannot inherit the cached tail, and uncertain mutations forbid reuse. The advertised stream timing limit preserves honest compatibility.
- **Pinned by**: [runtime §3.9](./rustack-workflows-20-lambda-runtime-design.md#39-invocation-scoped-log-windows), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26.


## D35 — Keep per-call pool cleanup settlement above dispatch workers

- **Context**: the Docker Executor must retain provider invocation permits through cancellation cleanup, including response-channel closure and pool worker/root failure.
- **Alternatives considered**: dropping an invoke future only requests cancellation; using a response-channel close as cleanup proof loses physical ownership; worker-local settlement senders disappear during panic; reusing an invocation deadline can skip cleanup once that deadline expires.
- **Decision**: expose an exclusive PoolInvocation with cancel-safe response waiting and a separate resumable cancellation-settlement operation; retain the sender beside the environment Retirement record above JoinSet and through emergency recovery.
- **Why**: upper-layer admission can await a positive reusable/retired proof without owning or aborting lower-layer resources. Failed supervision remains uncertain even after physical recovery; queued no-allocation rejection can settle cleanly.
- **Pinned by**: [runtime §4.5](./rustack-workflows-20-lambda-runtime-design.md#45-supervised-pool-invocation-receipt), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. This receipt does not enable production Docker or complete Executor/provider integration.


## D36 — Compose Docker execution through the retained pool and trusted readiness port

- **Context**: the Lambda provider already captures immutable revisions and supervises invocation permits; the Docker environment/pool owners now expose reusable/retired settlement proof.
- **Alternatives considered**: HTTP loopback invocation duplicates protocol/admission; a per-call Docker client loses bounded ownership; advertising readiness from a connected socket or a directory lease bypasses required startup qualification; rewriting handler outcomes into synthetic JSON breaks byte fidelity.
- **Decision**: implement the existing Executor using one pool call and independent settlement, with raw outcomes/resolved-version headers and one base64 tail encoding; inject a bounded synchronous readiness port from the trusted startup coordinator.
- **Why**: actual code execution and provider response composition can be verified independently while ordinary CLI selection remains unavailable until all production reconciliation/probes are implemented. The readiness interface shares the existing trusted Rust custom-Executor boundary and is never supplied through AWS requests. Pool/environment quota rejection is distinct from Docker wire saturation that may occur after side effects; only the former maps to429, while both retain no-replay behavior.
- **Pinned by**: [runtime §4.6](./rustack-workflows-20-lambda-runtime-design.md#46-docker-executor-and-provider-composition), [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. No new dependency is required; this does not close M0 startup/platform acceptance.


## D37 — Qualify actual kernel resources through the retained environment path

- **Context**: production readiness must verify the daemon's real container limits without trusting declared create fields or operator user images to attest themselves.
- **Alternatives considered**: info/inspect-only checks do not observe process isolation; a new one-off container owner duplicates uncertain-create/cleanup semantics; shell probes require image tooling and leak text; a network name can be retargeted after inspection.
- **Decision**: pin qualified bridge selections by full ID and execute a bounded Rust probe mode from the operator-pinned relay image as a private Lambda revision under EnvironmentManager; issue private report proof only after exact kernel validation and joined retirement.
- **Why**: probes consume the same policy/runtime path they qualify and retain existing cleanup ownership. Resource and network proofs remain independent from historical mutation reconciliation and cannot open readiness by themselves.
- **Pinned by**: [runtime §4.7](./rustack-workflows-20-lambda-runtime-design.md#47-startup-network-qualification-and-actual-resource-probe), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Promote the already-resolved runtime serde_json dev dependency to a normal workspace dependency for bounded typed report serialization; no new library/version is introduced.

## D38 — Persist container intent before possible creation and identity before deletion

- **Context**: in-memory uncertain-create receipts do not survive process death; an absent name cannot exclude a late daemon commit.
- **Alternatives considered**: label-only scans miss pending creation; negative lookup as proof loses ownership; always rejecting dirty restarts cannot reclaim positively identified resources; an unbounded append log adds retention and replay complexity.
- **Decision**: use a bounded atomic directory snapshot of container identities, publish before create, persist a positively verified full ID before removal, and erase only after defensible settlement. Historical unknown absence remains unresolved and blocks its relay.
- **Why**: the same retained directory capability protects filesystem transactions and daemon ownership; interrupted recovery is repeatable without replaying create or touching foreign resources. This is a container-specific part of full reconciliation, not production readiness.
- **Pinned by**: [runtime §4.8](./rustack-workflows-20-lambda-runtime-design.md#48-durable-container-intents-and-restart-reconciliation), [implementation plan](./rustack-workflows-90-impl-plan.md).
- **Date**: 2026-10-26. Reuse existing nix/sha2/hex dependencies and the bounded ownership worker pool.


## D40 — Generate exact JSON epoch timestamps before the Step Functions API

- **Context**: Step Functions uses AWS JSON1.0 numeric timestamps; default chrono serde emits strings, and the existing optional KMS float helper mishandles negative fractions.
- **Alternatives considered**: binary f64 arithmetic introduces avoidable rounding; copying the KMS helper preserves its negative-epoch defect; workspace-wide arbitrary_precision changes unrelated payload semantics; hand-editing generated files cannot survive regeneration.
- **Decision**: resolve timestamp-format precedence in the generator and emit a JSON-specific RawValue decimal adapter, normalized by checked bounded arithmetic to millisecond precision toward zero. Select strict model generation for Step Functions: documented/nonexhaustive DTOs, required collections, typed builders, redacted diagnostics and no blanket ignores.
- **Why**: SDK wire dates are numeric and exact at the promised precision, including negative/exponent/range edges, while other protocols retain their bindings. Oversized exponents are classified with bounded work. Serde-disabled output gets neither duplicate Debug nor unused codec modules.
- **Pinned by**: [API §1.1](./rustack-workflows-10-api-design.md#11-executable-timestampmodel-generation-contract), [verification](./rustack-workflows-70-verification-plan.md), [implementation plan](./rustack-workflows-90-impl-plan.md).
- **Date**: 2026-10-26. The model reuses workspace serde/chrono/typed-builder; the excluded developer generator adds the already-workspace [Tokio1.54.1](https://docs.rs/tokio/1.54.1/tokio/attr.main.html) with explicit local-IO features and independent audit/deny. No Step Functions endpoint or engine is advertised by the model alone.

## D41 — Terminate HTTP ambiguity before typed registry dispatch

- **Context**: the new workflow service needs AWS JSON1.0 routing, scope/authentication and bounded typed IO before the registry/ASL engine can consume it.
- **Alternatives considered**: raw-JSON handler ports repeat parsing/limits; a default handler creates misleading service availability; suffix-based routing can leak unknown actions to S3; serializing an unchecked handler response can return a wrong operation or unbounded history.
- **Decision**: expose typed18-action transport ports with explicit validated scope/auth configuration, source-level budgets and operation-correlated bounded responses. Couple main-program registration to the real core adapter. Preserve accepted command ownership beyond HTTP cancellation.
- **Why**: protocol qualification is independently testable without claiming workflow execution; core keeps domain/compiler semantics and cleanup ownership. Encoded page budgets prevent successful responses from relying on unbounded allocation.
- **Pinned by**: [API §1.2](./rustack-workflows-10-api-design.md#12-typed-http-boundary-before-registry-wiring), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Reuse existing HTTP/auth/JSON/Tokio/async-trait dependencies. Unknown-action404 follows the current official common-error reference; no AWS error-precedence execution is claimed.

## D42 — Compile bounded paths against immutable validated payloads

- **Context**: the registry/compiler and every state evaluator need shared data semantics before the execution owner exists.
- **Alternatives considered**: a general JSONPath engine admits unsupported syntax; interpreting path strings on every transition repeats validation; arbitrary Value cloning/serialization can allocate far beyond the output quota.
- **Decision**: compile a bounded winnow subset into private typed segments; distinguish missing/single/selection results, borrow immutable payloads and materialize through a bounded writer. Validate result-target roots and prohibit array growth. Construct from borrowed trees only after structural/encoded checks; give the internal context separate byte/depth/member headroom while retaining normal limits on selected outputs.
- **Why**: one pure implementation pins data behavior for future compiler/engine consumers without coupling core to HTTP, an SDK or Docker. Original business input bytes survive for name idempotency.
- **Pinned by**: [ASL§2.1.1](./rustack-workflows-30-asl-design.md#211-executable-path-and-payload-foundation), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Reuse the already resolved winnow1.0.4 and shared JSON/bytes/serde dependencies. This is a semantic foundation, not engine acceptance.

## D43 — Compile Choice predicates and bound evaluation across the whole rule

- **Context**: branching semantics need validated literal/Path operands and short-circuit behavior before the execution actor interprets Choice states.
- **Alternatives considered**: arbitrary JSON inspection on each transition repeats validation; user regexes complicate resource bounds; binary64 coercion silently aliases large integers; per-child budgets let compounds multiply work without bound.
- **Decision**: compile a private typed predicate tree, preserve left-to-right short-circuit and missing-reference errors, normalize timestamps to instants, use a bounded Unicode glob and share one step/string-byte budget across the complete predicate.
- **Why**: a pure reusable evaluator pins semantics and makes local limits explicit before machine compilation and scheduling. It remains separate from transition-target ownership.
- **Pinned by**: [ASL§2.4.1](./rustack-workflows-30-asl-design.md#241-executable-choice-predicate-contract), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Reuse existing chrono, JSON and reference paths. Numeric integers use the closed±2^53 range; floats exclude its endpoints to avoid source-rounding aliases. Enable shared serde_json float_roundtrip for correctly rounded interior numeric lexemes; no arbitrary_precision or new external version is added. Timestamp fractions retain precision beyond nanoseconds. No complete Choice state/engine acceptance is claimed.

## D44 — Resolve Wait sources once before monotonic scheduling

- **Context**: absolute Wait timestamps truncate target fractions, while execution deadlines and timer ownership remain monotonic.
- **Alternatives considered**: sleeping inside a pure evaluator obscures ownership; repeatedly subtracting wall clocks follows NTP jumps; defaulting invalid dynamic values hides workflow errors.
- **Decision**: compile exactly one typed source and resolve it against validated data/context plus one supplied wall-time sample, returning a bounded relative duration. The future actor owns checked monotonic scheduling and generations.
- **Why**: static validation, dynamic failures and AWS whole-second target behavior can be pinned without a sleeping task or dependency on the HTTP caller lifetime.
- **Pinned by**: [ASL§2.4.2](./rustack-workflows-30-asl-design.md#242-executable-wait-source-contract), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Use existing chrono/std/reference paths. STANDARD local wait cap is365 days; overall execution deadlines are independent. This is source evaluation, not scheduler acceptance.

## D45 — Compile intrinsics with shared budgets and preserve format escape provenance

- **Context**: Parameters/ResultSelector and the machine compiler need one reusable, bounded expression implementation.
- **Alternatives considered**: host formatting/splitting gives different semantics; reparsing on every state entry repeats validation; decoding all format escapes before slot recognition reinterprets escaped braces; per-call budgets multiply work under nesting; infallible UUID helpers can panic on entropy failure.
- **Decision**: compile the12-call allowlist into a private AST, preserve literal formatting provenance, share work/text/materialization budgets and encode borrowed collection views before owning results. Inject a typed UUID source and use fallible OS entropy in the concrete convenience adapter.
- **Why**: fixed bounds and historical independent edge inputs make expression behavior reviewable before template/compiler consumers. Async execution owners keep entropy acquisition off the scheduler loop.
- **Pinned by**: [ASL§2.3.1](./rustack-workflows-30-asl-design.md#231-executable-expression-contract), [semantic memo](../docs/research/spike-stepfunctions-semantic-oracle.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Add normal dependency edges to already-resolved getrandom0.4.3/uuid1.26.0 without updating external versions. Formal scalar Format is selected over the historical cloud's object formatting extension.

## D46 — Compile templates and keep original state input outside resource phases

- **Context**: reusable Parameters/ResultSelector expansion must share limits, and result merging must target raw state input across an external call.
- **Alternatives considered**: per-field fresh budgets allow aggregate work amplification; building expanded Value trees before sizing defeats output quotas; merging into prepared parameters changes state semantics; owning a resource inside a data helper obscures cancellation.
- **Decision**: compile normalized collision-free templates, encode borrowed prepared views under shared budgets, and expose pure prepare/finish phases requiring the owner to retain original input.
- **Why**: one implementation pins exact transformation ordering before state compilation/execution; future actor ownership and callback provenance remain explicit.
- **Pinned by**: [ASL§2.2.1](./rustack-workflows-30-asl-design.md#221-executable-templates-and-data-flow-phases), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. Reuse compiled paths/intrinsics and bounded serde output; no dependency changes. This does not close machine compilation or resource execution acceptance.

## D47 — Compile sequential machines into immutable scoped indices

- **Context**: the registry/execution actor needs validated state graphs and precompiled operations without runtime map/string interpretation.
- **Alternatives considered**: a runtime ASL walker repeats validation; a DAG restriction rejects legal loops; suffix-only resource routing accepts unsupported integrations; installing partially compiled definitions breaks revision invariants.
- **Decision**: use one bounded side-effect-free compiler, strict sequential profile, lexical scoped StateIds, typed read views and immutable original-byte artifacts. Compile all control-flow targets and policies before graph acceptance.
- **Why**: the same artifact/error drives Create/Update/Validate; later actor work consumes validated IR and concrete adapter availability remains a composition-root decision.
- **Pinned by**: [ASL§1.1](./rustack-workflows-30-asl-design.md#11-executable-sequential-compiler-and-indexed-ir), [API](./rustack-workflows-10-api-design.md), [verification](./rustack-workflows-70-verification-plan.md).
- **Date**: 2026-10-26. No new external dependencies; complete M1 requires registry/actor/real Lambda evidence beyond compilation.
