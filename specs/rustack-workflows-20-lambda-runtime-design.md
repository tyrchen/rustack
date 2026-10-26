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

### 2.8 Local image resolution and selected platform identity

ImageResolver::new(DockerPort) exposes resolve_local(reference, architecture, ImageConfig overrides, deadline) → ResolvedImage. ImageMetadata also owns validated ImageExecutionDefaults (normalized inherited argv/workdir/environment/user); captures and snapshots retain these values, not only their hash. Debug redacts them and deserialization revalidates their bounds. It performs inspection only, never pulls, builds, reads credentials or mutates daemon resources. The deployment actor owns its bounded work slot and publication/CAS; later allowed-pull resolution must reuse this validation path. Cancellation or expiry of this read-only operation cannot publish a late result. Recheck the absolute clock before and after metadata validation/fingerprinting.

First inspect the requested reference once and capture its canonical, addressable daemon Id. On negotiated API≥1.49, inspect that captured immutable Id with a percent-encoded JSON OCI platform containing exactly os=linux and architecture=amd64/arm64, avoiding a second lookup of the mutable tag. Retain the addressable image_id and optional selected_manifest digest separately: a selected child Id returned by containerd inspection is not necessarily independently addressable for create. Every environment create supplies the captured image_id plus the explicit linux/amd64 or linux/arm64 platform parameter; it never substitutes an unqualified tag or assumes the child digest is a local image object. Older APIs retain single-platform inspection only with verified classic-store evidence. Validate the resulting OS/architecture, positive size≤10GiB and canonical SHA-256 Id. If a Descriptor is present, accept only OCI image-manifest or Docker schema2 image-manifest media types, canonical digest and a compatible optional platform. Reject index/manifest-list/attestation/unknown descriptors. A platform-selected containerd Id may be a child manifest digest rather than the legacy config digest. Validate that returned Id, but use the captured addressable parent Id for create and retain the selected descriptor digest for identity/hash checks. Never create from a parent index without the declared platform. Reject explicit nonempty Manifests in the descriptor-free fallback. Require matching classic GraphDriver and /info driver names from overlay2/aufs/btrfs/devicemapper/vfs/zfs/fuse-overlayfs, well-formed bounded DriverStatus rows and no io.containerd.snapshotter.v1 driver-type marker. Normalize missing/null DriverStatus to empty after matching classic evidence, since classic VFS legitimately reports no status rows. A missing Descriptor or nonempty GraphDriver alone is insufficient. Ambiguous/downlevel containerd resolution fails explicitly rather than choosing the daemon default platform.

RepoDigests has≤128 validated digest references. Repository identity comparison applies Docker familiar-name normalization: absent registry defaults to docker.io, index.docker.io aliases docker.io, and a one-component Docker Hub repository gains library/. Registry host comparison is ASCII case-insensitive; repository paths remain case-sensitive. The original request spelling determines the short-local URI extension before normalization. A requested digest must match the selected descriptor digest or an advertised parent RepoDigest for the same repository; otherwise fail inspection. For explicit registry-domain references with matching repository evidence, use that repository plus the selected descriptor digest as registry_manifest; a legacy descriptor-free image may use a matching manifest RepoDigest, choosing the requested digest or otherwise a deterministic lexical digest. Do not use a digest from an unrelated alias/repository. A short local reference always uses registry_manifest=None and local://Id, even if BuildKit has populated RepoDigests. The returned registry URI reflects daemon metadata, not a network publication/existence check. The captured addressable Id must remain available through the deployment/version/artifact lifetime. The deployment actor still needs an owned availability lease/pin policy; tag movement must never cause it to execute a newer image. Missing externally removed content fails explicitly. CodeSize/size_bytes uses the selected daemon-reported Size, whose containerd representation can be compressed content rather than unpacked size. The deployment actor must separately prove the10GiB unpacked-image quota before Active; this remains a task1.3 gate and is not established by the read-only local resolver.

Validate inherited ENTRYPOINT/CMD with the same argv limits as function ImageConfig, preserving omitted versus explicitly empty overrides. Docker's single-empty-entrypoint reset means no entrypoint. Compose the effective entrypoint+command, require its first executable argument to be nonempty, and reject NULs/bounds before publication; do not require a shell or bootstrap name. An absent/empty inherited working directory means Docker's root default; an explicit override still must be absolute. Inherited image Env is≤128 entries,≤4KiB per entry and≤64KiB total, with no NUL; image Env is distinct from the function's stricter4KiB environment budget. Validate a bounded ASCII environment key of1..256 bytes before `=` (or a key-only deletion marker); preserve ordered values for Docker semantics. Image USER is bounded≤256 printable ASCII bytes. Reject image-declared VOLUMEs in this local profile: automatic anonymous writable mounts would bypass rootfs/tmpfs policy. The eventual container builder must also disable image healthchecks and apply the specified nonroot-user policy.

ImageExecutionDefaults::plan(overrides) validates the complete composed argv, including combined inherited/overridden byte budgets, and returns an immutable redacted ImageExecutionPlan. The plan materializes ENTRYPOINT, CMD and WORKDIR; it never relies on daemon fallback. Encode a semantic empty ENTRYPOINT as the Docker reset sentinel [""], preserve a nonempty entrypoint, and always send the full command array even when empty. This prevents Docker from re-inheriting CMD when the request entrypoint is empty. No per-invoke inspect is permitted. Empty inherited argv can be supplied by valid function overrides, but malformed inherited strings/directories remain invalid even when no default executable exists. Reject saved image metadata without validated execution defaults; older metadata-only records require deployment resolution rather than pretending to be executable.

```text
┌─ read-only image resolution ─────────────────────────────────────────┐
│ bounded platform inspect → child identity + validated defaults       │
│                                      │ stable base fingerprint      │
└──────────────────────────────────────┼───────────────────────────────┘
                                       ▼
┌─ immutable ResolvedImage / captured revision / snapshot ─────────────┐
│ Id + platform + URI evidence + defaults (argv/env/user redacted)      │
│                                    │ captured function overrides    │
│                                    ▼                                │
│                         validated materialized plan                 │
└────────────────────────────────────┼────────────────────────────────┘
                                     ▼
┌─ future environment owner ──────────────────────────────────────────┐
│ create by selected Id; full argv/workdir; empty entrypoint=[""]    │
│ security/resource policy + fixed environment injection separately   │
└────────────────────────────────────────────────────────────────────┘
```

Fingerprint a versioned, explicitly ordered serialization of captured addressable Id, retained selected_manifest digest, Linux architecture, inspected size and normalized inherited execution fields (entrypoint, command, root-default workdir, ordered environment, user). Exclude tags, unrelated RepoDigests, labels, creation timestamps and map iteration order. ImageMetadata::execution_fingerprint centralizes bounded SHA-256 serialization; ResolvedImage construction/deserialization recomputes it and rejects inconsistent metadata, including valid-string execution-plan mutations. No raw argv/environment or metadata is logged. Function overrides remain captured separately in ExecutionRevision and do not change the base inspection fingerprint. Invalid overrides fail before daemon IO. Fake-daemon tests cover platform query/version fallback, every rejection, deterministic identity under tag/label/RepoDigest ordering changes and explicit-empty overrides. Real wire qualification must create/run the function and relay using identities returned by this resolver.

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

### 3.1 Shared runtime crate and control schema

Add the host-portable library and Linux relay binary in `crates/rustack-lambda-runtime`. Lambda core depends on its library; the runtime crate never depends on lambda-core, Squib, the AWS SDK or Docker. Its protocol module owns framing and validated message types, its session module owns the single-invocation state machine, and transport adapters own HTTP/stdio IO and supervision. Deliver these contracts before the Docker consumer.

Protocol version1 encodes one definite-length CBOR map with exactly `version` (unsigned1), `environment` (canonical nonnil UUID string) and `message` (adjacently tagged `kind`/`data` map). Unit messages omit data. All identifiers are canonical lowercase hyphenated UUID strings. The message kind and data schema are:

| kind | Direction | data fields |
|---|---|---|
| hello | Relay→host | architecture: x86_64/arm64 |
| ready | Relay→host | None |
| bootstrapReady | Relay→host | None |
| invoke | Host→relay | attempt (UUID), deadlineUnixMs (nonzero u64), functionArn (validated Lambda ARN), traceId (bounded printable ASCII), payload (CBOR byte string≤6 MiB) |
| invocationResult | Relay→host | attempt, outcome: success/functionError, payload |
| initError | Relay→host | payload |
| cancel | Host→relay | attempt |
| close | Host→relay | None |
| ack | Relay→host | operation: cancel/close; attempt is required for cancel and forbidden for close |

Unknown fields/variants, duplicate keys, absent required values and mismatched per-variant fields reject. The decoder preflights the complete bounded byte slice before Serde allocation: definite maps only, at most16 nesting levels and128 total map members; text≤2,048 bytes; text map keys≤64 bytes; no arrays, negative integers, tags, floats, indefinite strings/collections or trailing items. Byte strings are allowed only as a single payload field; its bytes are excluded from the64 KiB metadata budget. Enforce the6 MiB payload and6 MiB+64 KiB frame limits separately. UUID, ARN, trace and deadline validation runs again during deserialization. Bytes uses serialize_bytes; a CBOR array/base64 string is not a valid payload. Debug and protocol-error displays omit payload bytes and invalid peer input.

Read/write helpers operate on an owned reader/writer task. Clean EOF is distinct from a truncated prefix/body; a partial-frame timeout invalidates that transport rather than retrying from the middle. Callers supply an absolute IO deadline consistent with startup, invocation or idle leases. Cancellation of a helper authorizes retirement of that connection only; reconnect/replay is forbidden. The protocol has no registry credentials or arbitrary commands. Cancel Ack proves session invalidation, not termination of user code; the environment owner must still kill/reap the function before cleanup settlement.

The existing workspace [Ciborium0.2.2](https://docs.rs/ciborium/0.2.2/ciborium/) provides CBOR/Serde encoding after preflight. Byte payload serialization uses the existing Bytes implementation; no second byte-buffer abstraction is required. Tests cover every variant, binary payloads, fragmented transport, clean/truncated EOF, malicious lengths/depth/duplicates/unknown fields, metadata/payload limits, canonical identities, timed partial IO and redacted Debug/errors.

### 3.2 Runtime session ownership

RuntimeSession is the single owner of idle/queued/delivered/closed invocation state and the first-poll readiness signal. HTTP adapters validate routes/headers/body budgets before sending typed commands over a bounded mailbox; the relay adapter validates protocol/environment/attempt before submitting the same commands. Only one queued or delivered attempt and one waiting GET /next are retained per environment. Concurrent polls/invocations reject explicitly. The first valid GET emits BootstrapReady exactly once, even when no job is queued. Completing an unknown/expired/already completed attempt cannot consume another attempt's response sender. Init failure and cancellation close the session; cancelled environments cannot accept another invocation.

The native HTTP adapter keeps its127.0.0.1 ephemeral socket and public executor module reexports. The relay binds127.0.0.1:9001. Both use identical session metadata/result/deadline rules, and adapter shutdown joins all connection/session tasks. Native init timeout waits for first-poll readiness rather than treating process creation as readiness. The relay's outer process supervisor owns stdio/HTTP tasks and exits on malformed control input, EOF, parent lease expiry or closed session; no transport failure silently recreates a session.

The shared session API has a RuntimeSession owner, cloneable SessionHandle and InvocationTicket. start(idle lease) creates one actor with an eight-command mailbox; the owner exposes its handle, first-poll readiness and a take-once initialization-error receiver. submit(validated Invocation, command deadline) returns a ticket only after actor admission. next(command deadline) permits one outstanding poll and returns the delivered Invocation. complete(attempt, optional Invocation-Id, outcome, payload, command deadline) validates both identities and active deadline atomically before resolving the ticket. initialization_error(payload, command deadline) is legal only before any event delivery, reports once and closes the session. Transport adapters convert SessionError into their own HTTP/control outcomes without logging invalid values.

Invocation has validated attempt/ARN/trace/payload fields and an absolute monotonic deadline. Ticket wait returns a typed success/function-error/init-error result. Dropping an attached ticket, including cancellation of its wait/admission future, signals a dedicated per-attempt cancellation receiver. A ticket's cancel signal never targets a later attempt. Cancellation/expiry invalidates the session, resolves the current ticket/poll once, and forbids reuse. Concurrent calls receive Busy, stale/foreign completion receives UnknownAttempt without consuming active ownership, and RPC queue overload is explicit. A cancelled long-poll waiter may be replaced without losing a queued invocation. Result delivery to a disconnected ticket closes the session rather than accepting further work.

Offering an event through a oneshot channel is not delivery proof: retain the queued Invocation until next() consumes its response and synchronously acknowledges a per-poll receipt. A dropped response future closes that receipt and restores the queued event. Prioritize receipt processing before later mailbox commands so immediate completion observes Delivered. Before dispatching each selected mailbox command, synchronously reconcile any ready/closed receipt and recheck stop/attempt validity: biased select alone cannot order concurrently arriving messages on separate channels. Once the adapter receives an event, failed HTTP transmission requires retiring its environment rather than silently replaying; the receipt does not prove user-code execution.

```text
┌─ RuntimeSession owner ───────────────┐   ┌─ SessionHandle / adapter ─────┐
│ priority stop + retained join      │   │ submit / next / complete     │
└──────────────────┬─────────────────┘   └──────────────┬────────────────┘
                   │ stop watch                         │ bounded mailbox8
┌─ Session actor ───▼────────────────────────────────────▼───────────────┐
│ Idle ──submit──▶ Queued ──offer to poll──▶ Offering                   │
│                   ▲                         │ per-poll receipt        │
│                   └── receiver disappears ──┤                        │
│                                             └── consumed ─▶ Delivered│
│ Idle ◀──────── matching, active completion ────────────────────┘      │
│ stop / cancellation / expiry ─▶ Closed: fail pending ticket + poll    │
│ recheck attempt validity at offer, receipt and completion transitions │
└──────────────────────────────────────────────────────────────────────┘
    InvocationTicket drop → per-attempt cancellation watch
    Closed session → environment owner terminates external resources
```

The actor prioritizes owner stop, active attempt cancellation/deadline and idle lease expiry over mailbox traffic. Activity refreshes the idle lease only at valid session commands; a running attempt uses its own deadline. Each command envelope carries its absolute RPC deadline, checked again before state mutation. Shutdown signals the actor and awaits its stored join handle by mutable borrow, so a cancelled shutdown waiter cannot lose the task. Cache failed joins so repeated shutdown never reports false success. Owner Drop signals stop and leaves the actor to finish its in-process channel cleanup; adapters still own/await their HTTP tasks and external execution resources. The parent Tokio runtime must remain alive until normal service shutdown has joined these owners. Paused-clock and contention tests pin single admission/delivery/settlement, cancelled poll replacement, stale-completion isolation, old-ticket cancellation, init/readiness, idle/attempt deadlines, overload and dropped shutdown/admission waiters.

### 3.3 Shared HTTP adapter and Native startup

The runtime library's HTTP adapter binds only an explicitly supplied loopback SocketAddr (Native127.0.0.1:0, relay127.0.0.1:9001). RuntimeApiHandle exposes its address/session port, take-once initialization notification and cancel-safe joined shutdown. One owned server task retains RuntimeSession and a JoinSet of at most four HTTP connections. Stop/session closure ends admission, signals graceful HTTP shutdown under a five-second deadline and joins every connection, then joins the session. This lets an accepted initialization-error response reach the RIC before closure; timed-out connection futures close their sockets and fail settlement. Drop signals the retained supervisor rather than losing ownership. Failed supervision is cached and prevents a later successful shutdown claim.

HTTP parsing limits are compile-time constants:16 KiB header buffer/aggregate header bytes,64 headers,256 bytes per value and256 bytes per request target. Reject absolute-form targets, query strings, duplicate Invocation-Id headers and nonprintable header values. Routes match the entire method/path and canonical attempt UUID; extra path segments do not reach the session. GET bodies must be empty. POST bodies are raw bounded bytes≤6 MiB, collected with a five-second absolute deadline shared with command processing. Reject all HTTP trailers before session mutation; streaming responses are outside this profile. Content-Length over the limit rejects before reading the body. Header parsing has its own five-second deadline. GET /next waits within the configured session idle lease instead of a five-second business timeout. Responses to rejected requests close the connection; normal RIC keepalive remains supported.

Wrap the HTTP supervisor and every spawned connection future with an independent task-bound subscriber filter that suppresses Hyper/hyper_util DEBUG/TRACE regardless of the application's subscriber/RUST_LOG. Preserve allowed application span IDs/context and events through delegation. The initial Hyper1.11.1 source audit found partial chunk-trailer buffers in TRACE before application validation, so rejecting trailers alone cannot establish secrecy. The current Hyper1.12.0/Tokio1.54.1 stack retains this mandatory filter and the captured-secret regression. Capture a fragmented secret trailer under TRACE as well as ordinary header/body/target values. This shared adapter filter is separate from the mandatory Bollard filter at the future Docker boundary.

GET responses include Request-Id, Invocation-Id (the same synthetic attempt UUID), checked epoch-millisecond Deadline-Ms, requested Function-Arn and a syntactically valid local Trace-Id. This local profile generates a fresh request/attempt UUID for every Invoke rather than implementing event-source event identity across redelivery. If Invocation-Id is present on completion it must match the path and delivered attempt; omission stays compatible with older RICs. Mismatch uses400/InvalidInvocationId without echoing values, Busy maps409, unknown attempts400, expired requests408 and closed sessions503. Successful completion/init error returns202. Infrastructure traces contain bounded classes/counts only, not targets, headers or payloads. This follows the [current AWS Runtime API contract](https://docs.aws.amazon.com/lambda/latest/dg/runtimes-api.html).

Native starts the API/session before spawning the bootstrap, then waits under one initialization deadline for first valid GET /next, initialization error, process exit or caller cancellation. Spawned children remain owned and are terminated/reaped on every failed/cancelled startup branch; API/session tasks are joined before clean settlement. The handler budget is captured after successful first-poll readiness when dispatching the event, additionally bounded by the caller's absolute deadline. Initialization time cannot consume/extend an unbounded fresh handler/Task budget. Runtime API errors still require retiring the process before propagating failure. Keep the executor module's runtime_api reexports, replacing the old pending-map/queue implementation with the shared adapter rather than maintaining a second session engine.

### 3.4 Relay control supervisor and process entry point

RelayHandle owns one supervisor; Drop signals priority stop and leaves that supervisor owning API/control cleanup, while shutdown joins/caches its result without moving the join handle across awaits. The supervisor owns RuntimeApiHandle, dedicated control reader/writer tasks and bounded two-message input/output mailboxes. A queued output includes a write receipt; Hello, Ready, results and acknowledgements count as emitted only after the frame is flushed. Unexpected IO/task termination invalidates the environment without reconnect/replay. Every exit closes/joins the API and aborts/joins the in-process IO workers; external function termination remains the host environment owner's responsibility.

Relay state is Initializing→Idle→Busy(ticket)→Idle; cancellation or initialization/function failure enters Retiring. Retiring accepts Close and idempotent Cancel for the last attempt until its bounded cleanup lease expires; acknowledging another matching Cancel must not extend that lease. Unexpected session closure or attempt expiry invalidates the control connection and exits under joined cleanup; the host must retire the external environment on that transport failure. Emit Hello then Ready after listener creation, and BootstrapReady once on the first valid GET /next. Invoke requires Idle and a matching environment UUID; reject wrong-direction messages, overlapping calls, wrong-scope frames and mismatched cancellation IDs. Validate/convert deadlineUnixMs before session admission. Keep attempt ticket ownership while selecting result versus control input; a cancel-safe borrowed ticket result method must not drop/replay a pending result or permit polling an already consumed oneshot. Completion and Close/Cancel races settle once; Cancel invalidates/closes the session before its Ack, never claims external user-code exit. Close joins the API and flushes Ack before IO teardown. A reader that queued Close may finish without making the supervisor discard that queued Close.

Separate first-byte waiting from partial-frame IO: silence is bounded by the larger of the session idle lease and one hour (the validated wall-deadline ceiling), while receipt of the first byte starts the short control IO budget for the remainder of the frame. Session/retirement deadlines still stop an idle/busy relay sooner. Do not cancel/restart a partially read frame when adjusting leases. Output writes use the short absolute IO budget. Defaults are idle600s, IO5s and retirement10s; validate the same local ranges as their owner budgets.

The `rustack-lambda-runtime` executable is Linux-only and serves127.0.0.1:9001. Required private child environment is RUSTACK_LAMBDA_ENVIRONMENT_ID; optional RUSTACK_LAMBDA_RELAY_IDLE_SECONDS / IO_SECONDS / RETIRE_SECONDS forward the owner's validated configuration without adding a new user configuration loader. Reject invalid/noncanonical values without echoing them. Advertise the binary's compiled x86_64/arm64 architecture. Stdout contains control frames only; diagnostic errors use bounded static tracing classes on stderr.

Use safe OwnedFd clones of stdin/stdout with Tokio Unix pipe Receiver/Sender::from_owned_fd inside the entered IO runtime. The current Tokio1.54.1 constructors validate pipe/access mode and enable nonblocking IO. Do not use tokio::io::stdin or a blocking read task: cancelling its future does not stop the blocking read and can keep runtime shutdown waiting after a lease expires. Reject nonpipe descriptors/unsupported platforms explicitly. Unit tests use owned cancel-safe duplex streams; process tests exercise pipe EOF/Close and actual Linux builds precede Docker qualification.

The optional local `test-lambda-runtime-docker` target builds the relay image and explicitly runs the resource-gated process suite against the selected local Unix socket. Missing Docker/images fail this explicit target; ordinary workspace tests do not pretend to provide Docker evidence. The test-only Docker CLI supplies non-TTY binary stdin/stdout; the production executor still requires the separately supervised Docker attach client. Preallocate unique container names and a run label before allocation so setup failures retain reconciliation ownership. Always remove both owned names, reap the CLI process and verify no run-labeled containers remain, even when the assertions fail. Exercise Hello/Ready/Close before a function exists, control EOF/truncation, and an unchanged official RIC image's computed result, warm counter, handler error and busy cancellation. These relay checks do not establish SDK/provider, deployment/pool or full M0 acceptance.

### 3.5 Host control client and shared stream ownership

The runtime crate's client module exposes ClientConfig::new(environment, architecture, io_budget), ControlClient::start(config, reader, writer), next(absolute_deadline), invoke(Invocation, absolute_io_deadline), cancel(attempt, absolute_io_deadline), begin_close(absolute_io_deadline) and shutdown(absolute_cleanup_deadline). Configuration validates a nonzero IO budget≤60s; invocation metadata uses the same checked wall deadline conversion and validated Invocation types as the local session. ClientEvent distinguishes RelayReady, BootstrapReady, InvocationResult(attempt, RuntimeResult), InitError(payload), Cancelled(attempt) and Closed. Payload Debug remains redacted. The client owns protocol state; it never owns or claims termination of external containers.

Share the relay's control-stream workers with the host through an internal transport owner. It owns bounded two-message mailboxes, partial-frame parsing, flush receipts, stop signal and a retained supervisor. The reader stops after queuing its direction's terminal message (Close on relay input, Ack/close on host input), so normal peer EOF cannot overtake a queued terminal acknowledgement. First-byte silence and partial-frame IO remain separate budgets. Drop signals stop; shutdown joins/caches its supervisor by mutable borrow, preserves completed IO failures and awaits every aborted worker. A cancelled next() waits on a cancel-safe mailbox receive; it never drops the worker's partial parser. Relay API shutdown still belongs to RelayHandle, before transport retirement.

Host receive state is AwaitHello→AwaitReady→Initializing→Idle→Busy(attempt)→Idle. Validate environment and advertised architecture before accepting Ready; consume Hello internally and return RelayReady only after the ordered pair. BootstrapReady is accepted once after Ready. Invoke requires Idle and a fresh last-attempt identity. Success restores Idle; FunctionError or InitError enters Retiring. Only one result per attempt is legal; any invocation result proves delivery, after which InitError is causally invalid even during later calls/cancellation. Recheck absolute receive/send deadlines after await and before successful state mutation: an already-ready inner future can win Tokio timeout polling after expiry. Reject duplicate/out-of-order/wrong-direction/foreign-scope frames and mismatched result/ack IDs without echoing their contents; invalidate the transport without replay.

Set a local write-in-progress flag before awaiting a flush receipt, clearing it only after success. If a send future is dropped, that flag makes later business operations fail closed: the invocation may already have reached the relay. The retained writer still owns any partial frame. Cancellation accepts the active or just-completed last attempt, sends Cancel, and waits through next() for its matching acknowledgement. While Cancelling, at most one matching late result/init failure can be discarded before Cancel Ack; it cannot settle the cancelled call. Matching Cancel after completed/error settlement is permitted to cover completion/cancel races. No fresh Invoke is permitted after cancellation.

begin_close sends Close after the Hello/Ready handshake and records closure separately from receive state. Until Ack/close, continue validating already queued bootstrap/result/init/cancel events in their ordinary causal order. Ack/close is illegal before an outstanding Cancel Ack. shutdown completes the handshake if necessary, begins closure once, consumes permitted prior events until Closed, then stops/joins IO. A protocol/IO/deadline error invalidates the session; shutdown still joins transport ownership and caches failure. Dropped shutdown waiters retain ownership and a later call must not turn an uncertain write or failed join into success. The environment owner stops/removes the function independently and retains its allocation reservation through confirmed cleanup or quarantine.

```text
┌─ Host environment owner ───────────────────────────────────────────────┐
│ ControlClient: ordered handshake / active attempt / cancellation       │
│ next() may be selected/cancelled; invoke/cancel/close await flush       │
│                       │                                               │
│       ┌───────────────▼─ shared transport supervisor ────────────────┐ │
│       │ bounded frame mailbox2 ◀── retained partial-frame reader     │ │
│       │ bounded write mailbox2 ──▶ retained writer + flush receipts  │ │
│       │ stop / cached join outcome; abort and join both workers      │ │
│       └──────────────────────┬───────────────────────────────────────┘ │
└──────────────────────────────┼─────────────────────────────────────────┘
               demultiplexed Docker attach streams (no reconnect)
┌──────────────────────────────▼─────────────────────────────────────────┐
│ Relay transport / RelayHandle ── RuntimeApiHandle ── official RIC       │
│ Close Ack: API joined + frame flushed; external function still owned  │
└────────────────────────────────────────────────────────────────────────┘
```

Tests pair real relay/shared HTTP with the host client over fragmented duplex streams. Pin ordered readiness, actual HTTP result/error forwarding, warm attempts, wrong scope/architecture/order/IDs, duplicate outcomes, cancellation with a late result, Close with queued events, cancelled next/send/shutdown waiters and cached IO failures. Re-run real Linux relay/RIC qualification after extracting shared IO; a production Docker attach adapter is still required before executor acceptance.

### 3.6 Bounded Docker API boundary

The pinned [Bollard transport study](../docs/research/study-bollard-docker-transport-boundaries.md) supersedes the initial ordinary-pipe-client recommendation. Reuse its independently published generated `bollard-stubs` models/query types (`1.53.1-rc.29.3.1`, default features disabled); own the small Unix HTTP/upgrade adapter for the operations this feature needs. Ordinary Bollard response collection and its permissive log decoder do not enforce these local pre-materialization limits. This changes the transport implementation, not the Docker API or runtime protocol. No production shell/CLI transport, TCP/SSH fallback or TLS feature is introduced.

DockerSocket is a validated operator-owned unix:// absolute path≤4,096 bytes, without controls/NUL. Resolve/check its Unix-socket metadata under an IO deadline; never consult ambient DOCKER_HOST or discover another socket. DockerWire owns bounded business/cleanup mailboxes of32/8 commands and at most eight short management jobs: six ordinary slots and two reserved cleanup slots. Kill/remove/inspection/reconciliation use the priority cleanup lane; owner stop has its own signal. Blocking lifecycle waits use a separate eight-command mailbox and at most32 retained wait jobs, bounded to one per allocated environment by the environment owner. They cannot occupy management or kill/remove reservations: two waits on running containers must never prevent the kills needed to release them. Ordinary deployment/start work cannot consume the reserved cleanup slots. Cloneable ports submit typed request envelopes with an absolute deadline; Drop of a response waiter never authorizes replay or loses connection-task ownership. The environment/deployment actor retains names/labels and side-effect reconciliation above the wire layer.

```text
┌─ typed DockerPort ────────────────────────────────────────────────────┐
│ business mailbox32 ──▶ six short management jobs ──┐                  │
│ cleanup mailbox8   ──▶ two reserved cleanup jobs ──┼─▶ owned Unix HTTP │
│ wait mailbox8      ──▶≤32 lifecycle wait jobs ─────┘    drivers        │
│ priority owner stop ─▶ close admission + signal and join all jobs     │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ supervised handoff (release short slot)
┌─ environment / deployment owners ────────────────────────────────────┐
│ attach / following logs / pull streams retained under their existing │
│ allocation reservations; stop/join independently before wire shutdown │
└──────────────────────────────────────────────────────────────────────┘
```

The host boundary lives in rustack-lambda-core::docker. DockerWire::connect(socket, deadline) starts its retained owner before awaiting version negotiation; dropping connection startup signals that owner rather than losing request drivers. port() returns a cloneable DockerPort; shutdown(&mut self) stops admission, signals and joins requests, and caches failure. DockerPort exposes typed version/info/image inspection and container create/start/inspect/kill/wait/remove/list/attach methods; stream/network/pull methods land with their consumers. Endpoint templates are private. DockerKey validates bounded identifier/name syntax and all path components are percent encoded. Labels supplied by the environment owner remain authoritative for mutation/reconciliation authorization above this transport layer. Request Debug exposes operation class/byte counts, never target/body values. Each upgraded attachment joins its HTTP driver before handing off an owned stream; split(io_budget) supplies a strict DockerOutputReader and raw stdin writer for the retained environment pumps. Reader next(deadline) yields redacted typed stdout/stderr chunks≤16 KiB; its record deadline survives chunk boundaries, and dropping an in-progress read poisons that reader rather than restarting partial framing.

Each management request owns a fresh Hyper HTTP/1 connection to that socket, with no keepalive pool. One retained job owns connect, handshake, request, bounded response consumption and connection-driver termination through join. Use a single absolute request deadline≤600s and recheck the clock after awaits and typed deserialization. Bound socket connect, HTTP header parsing, upgrade negotiation and body/frame IO; do not leave a detached Hyper driver or count response headers as complete body success. Ordinary one-shot HTTP drivers use without_shutdown to recover/drop their owned IO, avoiding redundant half-close errors after Unix peer EOF; upgraded drivers join before transferring the socket. Cancellation/expiry closes that request's IO and joins its driver. Caller cancellation does not implicitly cancel/retry the service-side resource operation; preallocated identities and reconciliation handle uncertain create results. Environment cancellation retains an already-issued create under its independent bounded Docker IO budget, then cleans its outcome; caller Task expiry is not permission to drop that create future. An uncertain create cannot be declared clean solely because one immediate name lookup returned404.

Compile-time wire quotas: management request/response bodies≤1 MiB each; response header buffer≤16 KiB,≤64 headers and≤4 KiB per value; request target≤4 KiB; decoded metadata JSON depth≤32,≤4,096 aggregate members/elements and≤16 KiB per scalar string. The larger scalar ceiling deliberately permits Docker inspection/configuration metadata; stricter image argv/environment/identity rules still validate before domain publication. Reject duplicate JSON object keys, HTTP trailers, wrong content framing and oversized Content-Length before collection. Stream chunked bodies through a byte limiter before materialization. Do not forward raw daemon error text, response bytes, registry credentials or URLs into infrastructure diagnostics; expose bounded operation/status/error classes with redacted sources.

Use a reusable rustack-core::json preflight before generated-model deserialization: JsonLimits::new(bytes, container_depth, aggregate_members, string_bytes, key_bytes), validate(bytes, limits) and from_slice<T>(bytes, limits). Enforce the Docker key cap256 bytes in addition to the scalar ceiling. A linear lexical string pass bounds encoded escape expansion before Serde scratch allocation; a DeserializeSeed visitor validates depth/counts/decoded lengths and duplicate decoded keys without building a Value tree. Drop each object's bounded key set after preflight; only then materialize the requested typed model. Error classes distinguish invalid limits, size/depth/count/string/key bounds, duplicates, syntax and target schema without retaining untrusted error text. Tests pin decoded escaped-key duplicates, byte-based Unicode limits, root/container depth, aggregate arrays/maps, malformed numbers/escapes, input limits and panic-free bounded malformed input. These shared primitives may be reused by the ASL compiler, whose own profile/domain validation remains separate.

Negotiate API versions from a bounded /version response. Support the local1.44..1.53 profile: select min(daemon ApiVersion,1.53), require that selection≥daemon MinAPIVersion and1.44, and reject malformed/incompatible versions. Validate daemon Linux OS/architecture and required resource capabilities during startup. Use fixed typed endpoint templates and correct path-component/query encoding. Initial operation families are version/info, image inspect/pull, container create/start/inspect/kill/wait/remove/list, attach/logs, scoped network create/inspect/remove and bounded startup-probe exec. This is a deliberately scoped adapter; arbitrary user-selected Docker paths/methods/headers never reach it.

Attach establishes the versioned HTTP upgrade before relay start. Its owner retains the Hyper driver until successful upgrade transfer, then owns the upgraded stream and supervised read/write pumps through shutdown. Attach, following logs and pull-progress jobs release their management slot only after transferring the stream **and** retained driver supervision into an owned response. A dropped/failed handoff signals and joins that connection's cleanup. Long-lived stream owners do not retain management slots: they are bounded by the existing environment reservations (control plus function-log streams, up to32 environments) or deployment reservations (pull streams). Thus warm environments cannot starve inspect/kill/remove admission. Do not pass streams through the ordinary Bollard LogOutput decoder. Require a non-TTY multiplexed eight-byte Docker output header: stdout1/stderr2, zero reserved bytes and declared payload≤8 MiB. Reject console/stdin/unknown channels, malformed/truncated headers/payload and oversized declarations before allocating. Stream payload in≤16 KiB chunks instead of buffering a whole Docker record. Docker record and CBOR frame boundaries are independent. Route relay stdout only into the bounded ControlClient byte stream, drain relay stderr into bounded diagnostics storage without logging its raw content, and preserve raw stdin writes separately. A read failure invalidates the environment without reconnect/replay. Missing Close Ack or uncertain stream cleanup cannot release an environment reservation as clean.

Function logs use the same strict outer framing with their own retained owner and bounded stdout/stderr rings, never sharing a control mailbox. Drain/count/drop excess output instead of blocking the handler. Control and log readers distinguish first-byte/idle waiting from partial-record IO; once a record starts it has the short control IO deadline. Stop signals must interrupt silent or flooding streams. Pull progress is bounded JSON-line streaming,≤64 KiB per line and≤4,096 bytes per displayed error, with the configured total pull deadline; discard progress after extracting bounded status/counts and never aggregate the whole layer-progress stream.

Mandatory task-bound diagnostic filtering covers Hyper/hyper_util and Bollard DEBUG/TRACE during construction and polling, including every spawned IO task. Keep the model-only production path and its filter explicit; the P0/full-client compatibility probe may still use Bollard and must not reintroduce verbose payload logs. This refines W01 rather than waiving it. Fake local Unix daemons must test oversized/chunked bodies, duplicate/recursive metadata, headers/trailers, malformed multiplexed lengths/reserved bytes/channels, truncated EOF, slow partial records, dropped waiters and TRACE-secret capture. Real daemon tests must prove 6 MiB-class control payload fragmentation, normal Close/Ack and no owned allocation left behind. Startup/limit enforcement and actual SDK executor acceptance remain task1.3–1.4 gates.

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

Use the rechecked Bollard0.21.1 generated-type/API contracts through model-only bollard-stubs; the bounded transport decision and quotas are authoritative in §3.6/D19. Reuse Hyper/Tokio/Bytes/ciborium. TLS transport is N/A for this release; future TLS must use rustls + aws-lc-rs rather than enabling a ring-backed SSL default.

## 8. Cross-references

- ← [PRD](./rustack-workflows-00-prd.md), [existing runtime spike](../docs/research/spike-cargo-lambda-runtime-execution.md)
- → [integrations](./rustack-workflows-40-integrations-design.md), [operations](./rustack-workflows-60-operations-design.md)
- ↔ [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)
