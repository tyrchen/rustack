# Workflows 20: Docker Lambda Execution Environments

Status: implementation contract; relay feasibility validated with platform qualifications · Depends on: [PRD](./rustack-workflows-00-prd.md), [Lambda hardening](./rustack-lambda-hardening-design.md)

## 1. Ownership and execution contract

Extend `rustack-lambda-core::executor` with DockerExecutor implementing the existing Executor interface. Lambda provider remains the only invocation entry point: HTTP Invoke, Step Functions and future event sources share version resolution, reserved concurrency, 32 global execution permits, default 8 permits per logical function, and 128 accepted asynchronous-work slots. Bridges must not bypass the provider by invoking an executor directly.

Add a small Linux `rustack-lambda-runtime` relay artifact. It serves the standard Runtime API inside the execution environment and exchanges private control messages over Docker attach. Extract reusable RuntimeSession state logic: Native retains its host TCP adapter, while Docker uses the environment-local adapter. Do not embed RIE or rewrite user images.

```text
┌─ Rustack host / Rustack server container ───────────────────────────┐
│ Lambda provider → WorkSupervisor → DockerExecutor/PoolActor        │
│ immutable revision                  bounded commands / JoinSet    │
│                                       ▼                           │
│ Docker Engine API client: Unix socket, API negotiation             │
└─────────────────────────────┬──────────────────────────────────────┘
            Docker attach stdin/stdout: non-TTY, framed CBOR
┌─ Docker daemon trust boundary▼─────────────────────────────────────┐
│ ┌─ one execution environment / independent network namespace ─────┐│
│ │ relay container                 function container              ││
│ │ RuntimeSession actor            original user image digest       ││
│ │ 127.0.0.1:9001 ◄── Runtime API ─ RIC / custom bootstrap           ││
│ │ owns netns                      network=container:<relay-id>    ││
│ └──────────────────────────────────────────────┬───────────────────┘│
│ no published Runtime API ports                 │ SDK → Rustack     │
│ private user bridge / operator network ────────┘ gateway endpoint  │
└───────────────────────────────────────────────────────────────────┘
```

The two containers share only the network namespace, not PID/mount namespaces, Docker sockets or host directories. Access to one's own Runtime API is intentional; other environments cannot reach that loopback listener. Do not publish Runtime API ports, use host networking or authenticate by NAT source address. Host and server-in-container modes use the same control path. Configure DNS/extra-host entries on the relay/netns owner; Docker rejects these options on a function using network=container:<relay-id>. [Docker networking](https://docs.docker.com/engine/network/) documents the foundation; the entire composition remains subject to actual platform tests.

Cost: two containers per warm environment. Each relay has a separate 64 MiB memory, 64 PID and 0.1 CPU cap. A 32-environment limit therefore permits at most 64 environment containers, with aggregate memory admission including relay overhead. Record the tradeoff in D2. If P0 invalidates the composition, revise this design and rerun its gate; do not silently substitute an exposed host listener.

## 2. Image resolution and deployment lifecycle

### 2.1 Immutable artifacts

Replace mutually inconsistent execution-request Option fields with `DeploymentArtifact::Zip(ValidatedArtifact)` or `Image(ResolvedImage)`, preserving public wire schemas. ExecutionRevision captures function resource UUID, resolved version, artifact/digest, architecture, ImageConfig, environment fingerprint, memory, timeout, ephemeral storage and backend policy. Use typed-builder for multi-field construction.

Keep the caller's requested function ARN/reference separately from the resolved numeric version. Runtime Invoked-Function-Arn preserves the invoked qualifier, while ExecutedVersion identifies the actual version. Environment reuse keys use resolved identity rather than the mutable alias string. Image CodeSha256 derives from resolved content metadata, not the requested tag text; CodeSize uses inspected image metadata with the local representation documented, and never claims to be a ZIP byte count.

ResolvedImage records requested reference, registry manifest digest when available, daemon content image ID, platform and inspection metadata hash. Invoke by content ID/digest, never by a freshly resolved tag. GetFunction.Code.ImageUri returns the requested reference; ResolvedImageUri returns repo@digest for registry images. A local unpushed image uses the explicitly local extension `local://sha256:<content-id>`; never invent an ECR manifest digest. Snapshots preserve this distinction; restoring such an image on another daemon requires importing the same content ID.

References are≤1,024 bytes and follow a bounded OCI/Docker repository/tag/digest grammar. Reject URL schemes, userinfo, controls and shell syntax. Local short names and non-ECR registries are advertised local extensions. Unchanged ECR references work with preloaded images or an allowed pull policy; Rustack does not emulate ECR.

Pulling is disabled by default. Enabling it requires an explicit registry allowlist and total timeout. The daemon performs network pulls. Registry credentials must be supplied through operator configuration; never automatically read host AWS credentials or execute shell credential helpers. Allowlist enforcement governs references submitted by Rustack; daemon-side mirror/DNS/redirect policy remains an operator responsibility and is not an SSRF guarantee.

### 2.2 Create, update and publication

Create(Image) rejects Runtime/Handler, defaults omitted Architectures to [x86_64], requires exactly one architecture after normalization, and rejects explicitly empty or multi-element lists. It checks mutually exclusive Image/ZIP/S3 code sources, and validates ImageConfig/environment. Reserve function identity and a deployment-work slot, return Pending, then resolve/pull/inspect on a bounded worker. Validate Linux OS, architecture and executable entry configuration before atomically making the revision Active. Failed deployments retain Failed plus bounded reason/code for inspection; Invoke rejects Pending/Failed. Deployment resolution requires available Docker; disabled execution never yields a falsely executable image revision.

Validate function MemorySize in 128..10,240 MiB, Timeout in 1..900 seconds, and EphemeralStorage in 512..10,240 MiB before allocation. User environment has at most 128 pairs; keys are 2..64 ASCII bytes matching `[A-Za-z][A-Za-z0-9_]+`, individual values are≤4,096 bytes and the total remains≤4 KiB. The key/count limits are local restrictions. Function name/qualifier/artifact validation remains governed by Lambda hardening. Parse error payloads only within their 32 KiB allowance; stackTrace has≤100 entries, each≤1,024 bytes, also constrained by that total.

UpdateFunctionCode keeps the previous revision usable while LastUpdateStatus=InProgress. Success atomically replaces latest; failure preserves the old digest/config and exposes a failed-update reason. RevisionId compare-and-swap prevents concurrent overwrite. Initial release rejects Image Create/Update with Publish=true using InvalidParameterValueException: asynchronous resolution cannot safely return a published version within the control deadline. This is a documented local limitation. A separate PublishVersion succeeds after Active/Succeeded and captures the resolved image and configuration.

Tag movement never changes deployed code. Published versions are immutable; aliases move references. Update/delete/recreate cannot reuse an old environment. In-flight calls keep their captured revision. Reject package-type changes. DryRun validates input, function/qualifier and permission shape without starting a container or executing code.

### 2.3 Platform and entry configuration

Map x86_64 to linux/amd64 and arm64 to linux/arm64. Guarantee native architecture only. Reject cross-architecture calls unless the operator explicitly enables emulation; do not promise emulation performance/parity. Registry manifest lists select exactly one matching platform and pin its child manifest digest. Inspect local image architecture rather than trusting the declaration.

Inherit Dockerfile ENTRYPOINT, CMD and WORKDIR, then override only explicitly supplied ImageConfig fields. Missing and explicitly empty arrays differ. WorkingDirectory is an image-internal absolute path≤1,000 bytes, not a host path. Each argv list has≤1,500 elements, each≤4,096 bytes and the combined argv bytes≤64 KiB locally. Pass JSON argv directly, never sh -c. Do not force CMD=bootstrap. P0 must prove official Lambda images enter their RIC path when AWS_LAMBDA_RUNTIME_API is set.

### 2.4 Invocation capture and resource identity

A function receives a nonnil random FunctionResourceId on creation, independent of its name, version and configuration RevisionId. Updates and publication preserve that resource identity; deletion followed by recreation receives a new one. Store mutation cannot change it. Snapshots preserve it; importing an older snapshot without this field assigns an identity once for the restored resource, never once per invocation. An explicitly malformed or nil saved identity rejects restore.

The provider captures the resource identity, resolved revision, complete ImageConfig (including omitted versus empty argv) and ephemeral storage before submitting work. The Runtime API invoked ARN is the function's canonical ARN with the caller's requested qualifier, if any. Alias invocation therefore reports `:prod` to the handler while ExecutedVersion reports the resolved numeric version. An unqualified request reports an unqualified ARN; explicit `:$LATEST` remains explicit. Pool reuse uses resource and resolved execution configuration, never the invoked alias. Requests' Debug representations redact environment values, payload, archive bytes and image argv; fingerprints may consume those values internally but must not be logged.

P1 task1.1 lands this capture contract before the artifact-enum and cooperative-cancellation consumers. Its regression tests cover delete/recreate isolation, snapshot identity preservation and rejection, alias versus resolved version, configuration capture and redacted diagnostics. These foundation checks alone do not close M0.

### 2.5 Captured artifact representation

InvokeRequest carries one DeploymentArtifact instead of independent package-kind/path/archive/image options. A Zip variant owns a cloneable ValidatedArtifact lease: its extracted root, raw Bytes and content hash refer to the same successfully staged archive. Only FunctionStore issues leases after ownership and filesystem validation. The last store/invocation lease owns directory cleanup, so dropping the store cannot remove code still used by an invocation. Staging shares Bytes instead of retaining a second archive allocation.

Validate each revision's normalized settings before downloading or staging its code. A code-source fetch has a hard 30-second deadline before staging; a paused-clock regression verifies stalled fetches leave no artifact. A newly registered ZIP belongs to an unpublished-artifact guard until the resource insertion or revision compare-and-swap succeeds. Failure, conflict or cancellation drops the guard, unregisters that new artifact and releases its directory; committed artifacts are unaffected. Snapshot restoration holds all new guards until the entire candidate store validates and commits. A rejected restore releases its new code without changing live resources. Hashing runs on the staging worker, not under a store mutation lock. Native's child supervisor retains an artifact lease until the child exit is reaped, including after the requesting store is dropped.

```text
Provider / restore          Store + staging worker            Artifact ownership
 │ 1. validate settings            │                                  │
 │ 2. stage shared Bytes ─────────▶│ private directory + digest         │
 │◀── unpublished guard ──────────│─────────────────────────────────▶│
 │ 3. validate record + insert/CAS │                                  │
 │── failure/conflict/cancel ──────┼── drop guard → unregister → remove │
 │── success → commit guard ──────▶│ store retains artifact lease       │
 │ 4. capture invocation ─────────▶│ clone validated lease ────────────▶│
 │                                │ native child also retains lease   │
 │ 5. store/request release        │ child exit → last lease → remove  │
```

An Image variant carries ResolvedImage, never a mutable tag alone. Its validated metadata contains the requested ImageReference, daemon SHA-256 content ID, optional repository/manifest reference, Linux architecture, inspection SHA-256 and inspected size. Digests use exactly 64 lowercase hexadecimal digits; inspected image size is bounded to 10 GiB locally. ImageReference accepts bounded Docker repository components, tags, SHA-256 digest suffixes and explicit DNS/IPv4/bracketed-IPv6 registry ports; URLs, userinfo, controls, invalid separators and empty components reject. Repository names are≤255 bytes and the whole reference≤1,024 bytes. Registry DNS labels are≤63 bytes; explicit ports use at most five decimal digits in1..65,535, an advertised local restriction. Bracketed IPv6 uses hexadecimal/colon syntax, excluding zone identifiers and dotted mapped forms. Follow the [distribution reference grammar](https://github.com/distribution/reference/blob/main/regexp.go), implemented with the pinned winnow parser rather than shell parsing. References are parsed again on deserialization. Missing resolution metadata cannot become an executable image; an existing metadata-only image record fails invocation explicitly until its deployment is resolved.

Artifact replacement does not change public AWS wire shapes. Native and Squib accept only Zip and consume its lease; Docker accepts Image in M0 and adds Zip in M4. The resolved architecture is a closed x86_64/arm64 enum, not an unchecked list. Capture validates numeric settings and full image overrides before work admission. No per-invoke image inspection or pulling is introduced by this contract.

### 2.6 Execution revision ownership

ExecutionRevision groups resource/revision identity, resolved version, artifact, architecture, runtime/handler, complete image overrides, environment, timeout, memory, ephemeral storage and selected backend policy. InvokeRequest carries an Arc of that captured revision plus the requested invocation ARN, payload Bytes and log-capture flag. Cloning a request shares revision storage; aliases and subsequent function updates cannot modify captured configuration. Runtime API metadata uses the requested ARN while backend results use the revision's resolved version. Warm identity includes the captured backend policy as well as resource/configuration identity. Debug on both types redacts user environment, argv, code and payload data. Native/Squib consume the same revision object.

### 2.7 Cancellable invocation and supervisor contract

Provider start_invocation returns an InvocationHandle after shared admission. The handle exposes its attempt UUID, wait, cancel, cancel_and_wait(cleanup deadline) and detach. Waiting returns typed ExecutorError outcomes; the existing HTTP Invoke maps these through its existing Lambda service error conversion. Event Invoke detaches only after admission succeeds. Dropping an attached handle signals cancellation, never aborts the worker. A wait deadline may return before cleanup; cancel_and_wait reports success only after cleanup and permit release. Its timeout returns CleanupFailed while ownership remains with the supervisor.

InvocationContext carries the same attempt UUID to the backend/Runtime API, an absolute monotonic deadline and a dedicated cancellation watch channel. The total deadline starts at admission, includes queue/cold start and is bounded by any caller Task deadline. Handler timeout remains separate. Admission checks backend availability against the captured request before accepting work. Unsupported backend/artifact combinations cannot return an accepted Event response.

A lazily started WorkSupervisor actor owns the bounded submission mailbox, JoinSet and cancellation registry. Existing limits remain128 accepted jobs,32 executing jobs and default8 per logical function across versions. A priority shutdown signal cannot be starved by submissions. Draining closes admission, finishes accepted work, and requests cancellation if its deadline expires. The actor continues owning cleanup after a quiesce waiter is dropped or times out. Worker panic or uncertain cleanup stops admission and prevents successful quiescence. Supervisor Drop signals stop; normal shutdown awaits settlement.

Native cancellation kills and reaps the owned child, drains log tasks and releases its process reservation before signaling clean settlement. Successful instances rejected by a full/disabled warm pool also retire before invocation settlement. Admission rejects declared host-architecture mismatches before accepting an Event; executable-file validation remains asynchronous. Executor Drop initiates retained process cleanup rather than aborting child supervisors.

Squib cancellation must retain its startup/VM ownership and await VM shutdown; never cancel/drop SquibBuilder::spawn or Squib::shutdown as a cleanup strategy. Inspection of the pinned Squib0.2.0 facade shows startup and shutdown temporarily own join handles across awaits, so dropping those futures is not a join/cleanup proof. A caller deadline does not authorize losing those futures. Squib guest cancellation retires/poisons that VM rather than reusing a possibly busy guest. Idle and busy VM owners use the same retained shutdown path, including executor Drop. Cleanup owners capture the parent Tokio runtime handle so transfer does not require Drop to run in an entered runtime context; normal service shutdown must await cleanup before stopping that runtime. The configured Squib shutdown budget bounds waiters, never the owned shutdown future. Expiry returns CleanupFailed, retains shutdown ownership and prevents successful quiescence.

```text
Caller / HTTP            WorkSupervisor actor             Backend supervisor
 │ start ───────────────▶│ reserve / capture attempt             │
 │◀── handle + UUID ─────│ spawn owned worker ──────────────────▶│
 │ drop/cancel ──────────┼── watch signal ──────────────────────▶│
 │ wait deadline → error │ worker remains owned                 │
 │ cancel_and_wait ─────▶│                         kill / shutdown / reap
 │                      │◀── confirmed cleanup ────────────────│
 │                      │ release execution + accepted permits │
 │◀── clean settlement ─│                                      │
 │ cleanup deadline     │ retain worker or backend quarantine   │
 │◀── CleanupFailed ────│ prevent false successful quiescence   │
```

## 3. Runtime API and relay transport

Implement the four basic 2018-06-01 routes: GET invocation/next, POST invocation/{requestId}/response, POST invocation/{requestId}/error and POST init/error. Each environment processes one invocation at a time. Managed Instances concurrency, extensions, streaming and SnapStart restore are excluded.

Inject `AWS_LAMBDA_RUNTIME_API=127.0.0.1:9001` and revision-derived Lambda name/version/memory/region/log/task-root metadata. Reject reserved-variable overrides. User environment totals≤4 KiB with Lambda key grammar. Reject NUL values and the [reserved Lambda environment keys](https://docs.aws.amazon.com/lambda/latest/dg/configuration-envvars.html#configuration-envvars-runtime), plus the local AWS_LAMBDA_FUNCTION_TIMEOUT injection. PATH, TZ and AWS_ENDPOINT_URL remain configurable. Inject only explicitly configured local test credentials; do not inherit host AWS secret variables.

Control frames are 4-byte big-endian length plus CBOR envelope, protocol version 1. Maximum frame is 6 MiB+64 KiB, with metadata≤64 KiB and payload encoded as CBOR bytes rather than base64. Message variants are Hello, Ready, BootstrapReady, Invoke, InvocationResult, InitError, Cancel, Close and Ack. Include environment UUID and invocation-attempt UUID. Hello advertises protocol version and architecture; mismatch retires the environment. EOF, malformed frames or attach failure invalidate it. Never reconnect and silently replay an invocation. Relay stdout carries frames only; stderr carries bounded redacted diagnostics. Collect function stdout/stderr separately from its own container. Enforce an independent subscriber filter that rejects all Bollard events more verbose than INFO regardless of RUST_LOG; the selected dependency logs HTTP request/response bodies and trailing attach bytes at trace/debug. Test secret-bearing create requests and truncated attach frames under RUST_LOG=trace. Application-level redacted Debug alone is insufficient.

Create the relay with Tty=false, OpenStdin=true and StdinOnce=true. Establish bidirectional attach before starting it so the initial Hello cannot be lost; then start and await Hello/Ready before creating the function container. Demultiplex Docker stdout/stderr frames before decoding CBOR, preserve partial-frame buffering within the maximum length, and cap/control metadata before allocation. Relay protocol output never passes through a text logger. Shutdown closes the input only after Close/Ack or deadline expiry. P0 experimentally showed StdinOnce=false leaves the relay running after attach-input shutdown, while true delivers EOF and terminates the relay. Each environment owns exactly one attach connection; reconnect/replay remains forbidden. Keep attach IO in separately supervised read/write tasks so slow log collection cannot deadlock invocation completion or cancellation.

Reject indefinite-length CBOR collections, nesting deeper than16 and more than128 metadata members before materialization; payload bytes are the only large field. Control-message variants validate permitted fields and reject duplicates/unknown fields. BootstrapReady is emitted once, only after the first valid RIC GET /next; relay Ready cannot satisfy the function initialization deadline. The bounded transport is not permission to deserialize arbitrary recursive CBOR structures.

Runtime `/next` supplies Request-Id, Deadline-Ms, Invoked-Function-Arn and Trace-Id. Convert wall deadlines with checked arithmetic and enforce elapsed time using monotonic clocks. If the optional Invocation-Id header is supplied on completion, it must match; omission remains compatible with older RICs. Unknown, expired or duplicate completions cannot resolve another request. Validate method/path/header/body limits before touching pending state. See [Runtime API](https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html).

```text
Provider             Environment actor          Docker/relay             RIC
 │1. admit + revision       │                         │                    │
 │─────────────────────────▶│2. attach/start relay ───▶│ Hello/Ready        │
 │                          │3. start function ───────┼───────────────────▶│
 │                          │                         │◀── GET /next ────│
 │                          │◀── bootstrap-ready ─────│                    │
 │                          │4. Invoke(id,deadline) ──▶│─── event ────────▶│
 │                          │                         │◀─ response/error ─│
 │                          │◀── InvocationResult ────│                    │
 │◀── typed result ─────────│5. settle once / pool     │                    │
 │ cancel/timeout ─────────▶│6. invalidate attempt    │                    │
 │                          │ kill function → close relay → remove both   │
 │◀── cleanup settlement ───│ release or quarantine allocation reservation │
```

Ready means the relay is listening; bootstrap-ready means the RIC issued its first GET /next. They are different signals. Image deployment, relay start, function initialization, handler execution and cleanup have separate budgets. Synchronous Invoke's outer total includes cold start. Function handler timeout starts at event delivery and is additionally bounded by the caller's Task absolute deadline: cold start must not extend that Task deadline. Idle `/next` uses the environment idle/shutdown lease, not the short control-request timeout; this is bounded long polling.

## 4. Environment states, pooling and cancellation

State enum: Allocating → Initializing → Idle → Busy(attempt) → Idle. Any stage can become Retiring → Removed; failed cleanup becomes Quarantined. PoolActor owns its map and LRU. Bounded Docker IO futures run in a supervised JoinSet; never hold a shared-state lock across IO. Give cancellation a separate priority lane or reserved capacity.

Warm identity includes resource UUID, resolved version, artifact/digest, configuration fingerprint, architecture and backend. Never share across functions. Preserve 32 global warm environments and default 1 retained warm environment per revision. Busy allocations have the same independent global cap; excess concurrent environments retire after completion instead of growing the warm pool. Wait for environment capacity only within bounded queue/deadline budgets. Default idle lifetime is 600s. Init/protocol failures, OOM, timeout and failed invocations retire their environments.

Cleanup ordering: invalidate attempt → cancel relay job → stop/kill function → await exit → remove function → close/remove relay → release environment reservation. Normal execution permits survive through cleanup. If Docker becomes unreachable, retain container IDs and a quarantine allocation reservation against global capacity; the caller may receive failure while reconciliation remains owned by the supervisor. Retry cleanup after 1/2/4 seconds, capped at 30s between attempts. A dropped HTTP future cannot orphan containers. Unresolved cleanup at shutdown sets readiness=false, prevents successful consistency snapshot publication and produces nonzero exit.

Labels include persistent data-directory ownerId, runId, environmentId and function resource UUID. Startup may reclaim old runs only for the same owner after acquiring its directory's exclusive lock. Never delete containers belonging to another Rustack instance based on a generic Lambda label; never delete user images. Keep revision/artifact references alive until invocation cleanup finishes.

## 5. Isolation and resource limits

- Initial Docker transport is a local Unix socket only. Disable implicit TCP/SSH fallback. Explicit docker configuration requires a startup daemon/version/relay-image/resource-limit probe; failure aborts startup. Default remains disabled. Auto does not acquire a new Docker fallback; Image on native/squib/auto fails explicitly.
- Function containers use read-only rootfs, cap-drop ALL, no-new-privileges, default seccomp,≤256 PIDs, default 1 CPU and configured function memory subject to aggregate admission. Prohibit privileged mode, host PID/IPC/network, devices, Docker sockets and arbitrary host mounts. Preserve a nonroot image USER; override absent/root USER with the configured unprivileged UID/GID. Permission-incompatible images fail explicitly.
- `/tmp` is a size-capped tmpfs, 512–10,240 MiB. Its used memory counts toward the container memory cap, unlike AWS disk-backed ephemeral storage; advertise this limitation. ZIP code reaches a daemon-owned volume through a validated archive, not a host-path bind assumption. Mount `/var/task` read-only; never expose the project root.
- Docker runs operator-selected code; it is not a hostile multi-tenant isolation promise. Egress follows the configured Docker network. Without network policy, do not claim handler SSRF prevention. Registry pull allowlists do not restrict arbitrary handler networking.
- Continuously drain stdout/stderr into separate bounded rings,≤256 KiB each; LogType=Tail returns the final 4 KiB. Cap individual diagnostic lines at 16 KiB and errors at 32 KiB. Drop/count excess logs without blocking handlers. User stdout is intentional user output, not infrastructure tracing; do not promise automatic secret detection in arbitrary logs.

## 6. Responses, errors and ZIP support

`/response` yields original Bytes. `/error` yields Invoke HTTP200 plus FunctionError=Unhandled and bounded payload. Missing images, OOM, missing RIC, early exit, initialization and protocol failures produce distinct ExecutorError variants mapped to Lambda errors. Infrastructure failures must never become success payloads. ASL mappings are specified separately.

M4 adds Docker execution of provided.al2023 ZIPs using an operator-pinned OS-only image digest. Safely populate the code volume and explicitly execute `/var/task/bootstrap`; an OS-only image's runtime launcher is not the user's bootstrap. Validate Linux ELF architecture. Preserve Native/Squib ZIP behavior. Managed Python/Node ZIP runtime mappings are excluded; deploy images containing their RICs instead.

## 7. Rust engineering contract

Apply AGENTS.md § Error Handling/Async/Type/Safety: thiserror sources; retain async-trait for the existing dyn Executor and document object safety; concrete internal actors use ordinary async methods. Use Bytes/Arc revisions, validated artifact enums, redacted Debug, checked memory/deadline arithmetic, forbid unsafe, IO deadlines and supervised JoinSet tasks. Serde control envelopes use camelCase; Runtime API uses AWS headers. Validate raw lengths before parsing. Tests use actual RIC images and cancellation/failure matrices. Tracing contains IDs, durations and classes, not payloads. Avoid per-invoke pull/inspect. Public APIs document Errors and examples; operational docs cover both deployment topologies.

Use the surveyed bollard 0.21.1 API as a starting point and recheck before implementation; disable default features and enable pipe only. Reuse ciborium. TLS transport is N/A for this release; future TLS must use rustls + aws-lc-rs rather than enabling a ring-backed SSL default.

## 8. Cross-references

- ← [PRD](./rustack-workflows-00-prd.md), [existing runtime spike](../docs/research/spike-cargo-lambda-runtime-execution.md)
- → [integrations](./rustack-workflows-40-integrations-design.md), [operations](./rustack-workflows-60-operations-design.md)
- ↔ [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)
