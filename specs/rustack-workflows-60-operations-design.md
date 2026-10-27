# Workflows 60: Configuration, Operations and Snapshot Boundaries

Status: implementation contract · Depends on: [API](./rustack-workflows-10-api-design.md), [Lambda](./rustack-workflows-20-lambda-runtime-design.md), [ASL](./rustack-workflows-30-asl-design.md), [integrations](./rustack-workflows-40-integrations-design.md)

## 1. Extend the existing configuration system

Use `RUSTACK_CONFIG` and the existing YAML `environment`, `budgets`, `advertisedEndpoint` schema. Do not add a parallel configuration loader or mutate process environment. Add keys to the validated settings allowlist and add typed, camelCase budget fields. Process environment overrides YAML; defaults apply last. Reject unknown keys, malformed scalars, zero/excessive budgets and conflicting executor settings before listening. Configuration structs use typed-builder and validated newtypes.

The following is a **proposed configuration after implementation**, not a statement that the current binary supports these keys:

```yaml
environment:
  SERVICES: "lambda,stepfunctions"
  LAMBDA_EXECUTOR: "docker"
  LAMBDA_DOCKER_SOCKET: "unix:///var/run/docker.sock"
  LAMBDA_DOCKER_RELAY_IMAGE: "rustack/lambda-runtime:integration"
  LAMBDA_DOCKER_NETWORK: "rustack-integration"
  LAMBDA_DOCKER_PULL_POLICY: "never"
  LAMBDA_DOCKER_GATEWAY_ENDPOINT: "http://host.docker.internal:4566"
budgets:
  workflowOpenExecutions: 128
  workflowDispatches: 32
  workflowMaxDurationSeconds: 3600
  workflowSnapshotBytes: 536870912
  workflowHistoryBytes: 33554432
  workflowTotalHistoryBytes: 268435456
  lambdaDockerEnvironments: 32
  lambdaDockerMemoryBytes: 8589934592
```

Start with `RUSTACK_CONFIG=./rustack.yaml rustack`; the current CLI does not have a `--config` flag. Existing `--snapshot <name>` remains the persistence entry point. Installing this feature must not silently enable code execution: absent explicit selection, Lambda remains disabled.

### 1.1 Docker configuration

| Key | Default and validation |
|---|---|
| LAMBDA_DOCKER_SOCKET | `unix:///var/run/docker.sock`; operator may select Desktop/rootless socket; absolute local socket path≤4,096 bytes; no TCP/SSH fallback |
| LAMBDA_DOCKER_RELAY_IMAGE | Required when Docker enabled; bounded image reference, resolved and pinned at startup; no implicit pull |
| LAMBDA_DOCKER_NETWORK | Operator-owned user bridge name/ID, or a per-owner private bridge created by Rustack;≤128 ASCII bytes; explicit none opts into disconnected pure handlers with SDK callbacks unavailable; never host/default-bridge/arbitrary-container sharing |
| LAMBDA_DOCKER_PULL_POLICY | never / ifMissing; default never; missing image gives actionable error |
| LAMBDA_DOCKER_REGISTRIES | Required for ifMissing; comma-separated≤32 approved registry hosts, each≤253 bytes; exact host[:port] matching, no suffix matching |
| LAMBDA_DOCKER_REGISTRY_AUTH_FILE | Optional operator-owned YAML credential map by approved registry host;≤64 KiB,≤32 entries, secret fields redacted and permissions checked; parse as a secret-bearing type, never execute credential helpers |
| LAMBDA_DOCKER_GATEWAY_ENDPOINT | Optional operator-trusted HTTP(S) endpoint reachable by functions;≤2,048 bytes; no userinfo/fragment; never derive from a request Host header |
| LAMBDA_DOCKER_UID / GID | Default1000; nonzero bounded numeric IDs; only overrides absent/root image USER |
| LAMBDA_DOCKER_CPU | Default1; finite decimal CPU units in 0.1..16; no zero/unlimited value |
| LAMBDA_DOCKER_LOG_RING_BYTES | Default262,144;4,096..1,048,576 per stdout/stderr ring |
| LAMBDA_DOCKER_CROSS_ARCHITECTURE | false; true permits an explicitly configured daemon emulation path, outside guaranteed performance |
| LAMBDA_DOCKER_ZIP_IMAGE | Required only for Docker ZIP; provided.al2023 image pinned by content/digest; no implicit runtime catalog |

Retain LAMBDA_DOCKER_ENABLED as the existing alias for explicit docker selection; conflicts remain errors. Add configured local test credentials through the existing secret-bearing configuration mechanism; never print them or infer real AWS credentials from the host. The advertised gateway endpoint and the function-visible endpoint are independent; the former cannot automatically solve Docker networking.

Registry credential records accept only approved host, username≤256 bytes, password≤4,096 bytes or identityToken≤8,192 bytes, with explicit mutual-exclusion rules and secret wrappers. The credential-file path is an operator-controlled local path, loaded with a timeout and restrictive file-ownership/permission checks. The configuration file must not be included in generated artifacts, snapshots or user-code mounts.

### 1.2 Tunable budgets

All budgets are validated once; limits below are local defaults and safe configuration maxima. Static AWS payload/definition quotas are compile-time constants and cannot be raised by YAML.

| Budget field | Default | Allowed range / relationship |
|---|---:|---|
| workflowApiAccepted / workflowApiWorkers |32 /8|1..128 /1..32; workers≤accepted, root admission remains charged through ready replies |
| workflowPaginationBytes |64 MiB|8 MiB..1 GiB incremental shared snapshot storage, plus core allocation/history budgets |
| workflowRegistryMachines |256|1..4,096 current resources |
| workflowRegistryVersions |1,024|1..4,096, independently applied to retained definition revisions and immutable tag versions |
| workflowRegistryRecords |10,000|1..100,000 total retained execution records, including RUNNING and unsettled terminal work |
| workflowRegistryCommands |128|1..1,024 accepted commands, held through pending CPU preparation and unobserved replies |
| workflowOpenExecutions |128|1..4,096, including children |
| workflowEvaluations |8|1..128; retained blocking evaluation/entropy workers |
| workflowDispatches |32|1..1,024; released while waiting on child/callback |
| workflowLambdaAcceptedCalls |128|1..1,024; separate binary-bridge queue/running/quarantine ownership budget |
| workflowLambdaCleanupSeconds |10|1..60; initial cleanup observation only, never releases an unresolved invocation |
| workflowFrames / workflowFramesPerExecution |4,096 /512|1..4,096 /1..512; per-execution≤global; roots and retained ancestor leases count |
| workflowResultBytes / workflowResultBytesPerExecution |256 MiB /32 MiB|4 KiB..1 GiB /4 KiB..128 MiB; per-execution≤global; intermediate indexed aggregation slots |
| workflowMaxDurationSeconds |3,600|1..31,536,000; independent local ceiling even without ASL TimeoutSeconds |
| workflowTransitions |100,000|1..1,000,000; history may exhaust first |
| workflowSnapshotBytes |512 MiB|1 MiB..1 GiB; conservative copy and per-part encoded limit |
| workflowHistoryBytes / workflowTotalHistoryBytes |32 MiB /256 MiB|1..256 MiB /8 MiB..4 GiB; per-execution≤total |
| workflowCompletedExecutions |10,000|1..100,000; do not evict unexpired retained records |
| workflowRetentionSeconds |86,400|60..7,776,000; terminal history retention |
| workflowNameTombstones |100,000|1..1,000,000; always retain names 90 days |
| workflowPaginationTokens |10,000|1..100,000; token TTL 24h |
| workflowCallbackTokens / workflowCallbackTombstones |4,096 /100,000|1..65,536 /1..1,000,000; retained active+closed entries≥live tokens; closed-token TTL follows workflowRetentionSeconds |
| workflowCallbackCommands / workflowCallbackWorkers |1,024 /8|1..100,000 /1..128; commands include accepted actions and ready acknowledgments; workers bound retained mint entropy |
| lambdaDockerEnvironments |32|1..32 initially, including busy/idle/quarantined environments |
| lambdaDockerMemoryBytes |8 GiB|256 MiB..1 TiB; includes relay caps and all allocated environments |
| lambdaDockerPullSeconds |300|1..600; one absolute queue/connect/progress deadline, independent of metadata IO |
| lambdaDockerDeploymentJobs |8|1..32; queued plus running deployments;≤image preparation capacity and pin budget |
| lambdaDockerDeploymentSeconds |300|1..3,600; absolute queue plus preparation deadline |
| lambdaDockerImagePins |1024|1..8,192; preparations, live revisions/captures and quarantined aliases |
| lambdaDockerIoSeconds |5|1..60; independent bounded management/mutation and partial-control IO budget |
| lambdaDockerPullSeconds / lambdaDockerStartSeconds |120 /15|1..600 /1..60 |
| lambdaDockerCleanupSeconds |10|1..60; bounded by global shutdown remaining time |
| workflowDrainSeconds |5|0..60; bounded by global shutdown remaining time |

Deployment JSON/base64 workers have a fixed aggregate ceiling of four, retained through completed results and provider dispatch; overload returns429. Code bodies remain capped at96MiB, with only the designated inline ZIP span exempted from the1MiB metadata budget. Unknown fields and malformed/budget-exceeding JSON return static400 errors; worker failure is a static500 error. See [runtime §2.12](./rustack-workflows-20-lambda-runtime-design.md#212-side-effect-free-code-validation-and-bounded-deployment-json). These are compile-time safety ceilings, not YAML tuning fields.

Registry/execution mailbox capacities and per-tick quantum are compile-time implementation constants from the ASL design, not runtime tuning knobs. Function init/idle settings reuse existing validated Lambda configuration. CPU and log limits use the exact environment keys above. Effective memory admission may allow fewer than 32 environments. Lower quotas must reject admission or fail the affected execution explicitly, never silently drop data/work.

### 1.3 Persistent Docker namespace ownership

Acquire the existing configured data directory before constructing a mutating image owner. [Runtime §4.1](./rustack-workflows-20-lambda-runtime-design.md#41-persistent-directory-lease-and-namespace-capability) defines its descriptor lease, immutable checksummed owner UUID, fresh run UUID and filesystem trust rules. Acquisition has a positive absolute deadline≤60s and a fixed four-worker IO ceiling. Require an operator-controlled local filesystem with advisory locking, atomic rename and directory sync support; an unsupported filesystem fails acquisition rather than falling back to an unlocked namespace. ACLs must not grant untrusted namespace/ancestor replacement, and operators must not unlink/replace live lock files or data roots. Same-UID/root or Docker-daemon adversaries remain outside this local execution isolation promise.

The permanent lease.lock inode is never truncated, replaced or deleted during release. Last-descriptor closure releases exclusion; image scopes, pins, queued work and quarantined cleanup retain clones until their ownership settles. A failed/cancelled acquisition may leave bounded initialization files, which are validated on retry. A corrupt published identity fails closed and is never silently regenerated. Errors contain static classes without operator paths or identity values. No Docker effect is authorized until acquisition succeeds.

This capability alone is not a ready backend. Startup must separately reconcile prior runs and unresolved historical effects under the same persistent owner before accepting image execution. Do not treat one empty list/404 as proof that a prior uncertain daemon mutation cannot appear later. Shutdown keeps the lease until environment/deployment/image cleanup owners are joined; unresolved cleanup prevents successful quiescence/snapshot publication.

### 1.4 Network and kernel probe receipts

Startup must use [runtime §4.7](./rustack-workflows-20-lambda-runtime-design.md#47-startup-network-qualification-and-actual-resource-probe) to inspect an exact operator bridge and bind the full network ID before allocation. none is deliberately disconnected. Relay/function creation explicitly requests private cgroup namespaces; the pinned Rust probe checks current controller membership before reading normalized kernel limits. Host validation and joined retirement precede issuance of a private, exact-input resource proof; a final deadline/live-input check prevents late or revoked success. A serialized report alone cannot establish readiness.

`make test-lambda-probes` runs bounded parser/qualification/facade regressions. `make test-lambda-probes-docker WORKFLOWS_DOCKER_SOCKET=<socket-path> WORKFLOWS_ARCH=arm64 WORKFLOWS_NETWORK=<existing-user-bridge>` requires an explicit pre-provisioned operator bridge and the pinned fixture images; it runs both none and bridge cases without silently skipping either. Tests do not create/delete the operator bridge. Test provisioners retain their own bridge identity/ownership and remove only their known bridge after all attached probe environments settle.

Current actual execution evidence is on cgroup-v2 arm64 VZ and amd64/QEMU daemons. v1 normalization/private namespace behavior has parser and allocation-body coverage; an actual v1 daemon lane is still unmeasured. Network/resource receipts are implemented components; historical uncertain effects, registry pulls, endpoint reachability and the production startup coordinator remain mandatory before CLI Docker enablement.

### 1.5 Historical container records

[Runtime §4.8](./rustack-workflows-20-lambda-runtime-design.md#48-durable-container-intents-and-restart-reconciliation) defines the bounded containers.v1 snapshot and its safe staging file. Environment creation durably records identity before daemon issuance; known IDs are persisted before deletion. Failed durable erasure retains allocation capacity even when Docker returned a no-effect status or the container is absent. Never remove these files to bypass an unresolved startup: unknown absence can precede a late create.

Before ordinary admission, call reconcile_containers under the same DirectoryLease. It processes historical functions before their relays, leaves current-run records untouched and returns remaining/settled counts. Unknown404 remains unresolved; a positive exact identity is pinned durably before ID-only retirement. Repeating a cancelled/failed pass resumes the saved state. Foreign identity or corrupt/unsafe files fail closed. This operation covers recorded environment containers; private image aliases are reconciled afterward through the separate pin journal described below. Other mutation classes, allowed pulls and the complete coordinator remain required before production readiness.

`make test-lambda-recovery` covers bounded/corrupt files, restart transactions, cancellation, foreign resources and delayed commits. `make test-lambda-recovery-docker WORKFLOWS_DOCKER_SOCKET=<socket-path> WORKFLOWS_ARCH=arm64` provisions actual owned containers, releases/reacquires the directory under a new run and retires the historical pair. Failed actual qualification preserves durable ownership evidence rather than deleting the temporary data root.

#### Historical private image aliases

[Runtime §2.13](./rustack-workflows-20-lambda-runtime-design.md#213-durable-image-pin-intents-and-historical-alias-retirement) defines pins.v1/pins.pending under the same retained descriptor lease and serial transaction boundary. The image owner persists intent before Tag, commitment before publishing a live lease or deleting an alias, and erasure before releasing pin capacity. Failed storage retains ownership and quarantine through retry; a positive acknowledgment or exact inspection remains a retained Committing phase until its publication succeeds. Never remove these files to bypass unresolved effects.

After historical containers settle, reconcile_image_pins examines only previous-run records. An unknown404 remains unresolved, while matching positive identity can establish durable commitment. Confirm absent committed aliases before durable erasure; never replay Tag or remove operator source tags/digest IDs. The returned counters do not establish backend readiness. Both recovery operations require the same daemon that received the intents: before production admission, durably bind the directory to a validated stable daemon identity before the first mutation, and reject mismatched or unprovable historical bindings before any404-based settlement. This mandatory binding prerequisite is implemented as W217/D76 in the [review backlog](./rustack-workflows-93-review.md).

`make test-lambda-image-recovery` runs the codec, retained image-owner and scripted recovery regressions. `make test-lambda-image-recovery-docker WORKFLOWS_DOCKER_SOCKET=<socket-path> WORKFLOWS_ARCH=arm64` adds actual unknown-visible and committed-absent alias recovery across directory reacquisition. Tests retain/print the evidence root on failure, verify the original source tag and exact alias absence, and always attempt joined wire cleanup. Ordinary CLI Docker selection remains unavailable pending the coordinator and remaining M0 gates.

The W217 consumer uses [runtime§4.9](./rustack-workflows-20-lambda-runtime-design.md#49-durable-daemon-binding-and-connection-local-identity-fences): bind the raw wire port to the exact directory before constructing runtime owners or recovering records. daemon.v1 persists the canonical engine UUID; a missing binding with any journal record, corrupt binding or changed ID rejects startup. Never delete/reset it to bypass failure. Keep the operator engine-id unique and coupled to its Docker data root; cloning/resetting it while ownership is outstanding violates the local installation identity contract. Every bound request verifies identity on the same connection it uses for the operation, and cannot reconnect after a checked connection closes. A bound port retains directory exclusion through queued and in-flight work. This prerequisite does not grant production readiness.

Before starting any runtime producer, call [runtime§4.10](./rustack-workflows-20-lambda-runtime-design.md#410-recorded-namespace-retirement-and-complete-observed-inventories) reconcile_namespace using the exact bound port. It drains recorded containers before pins, retains unknown effects and audits the complete observed owner namespace without limit64 or slash-limited reference globs. Current-run work, unrecorded owned resources, oversized metadata or later nonempty observations reject qualification. Do not remove journals or auto-adopt these resources to bypass uncertainty. The all-images response retains the1MiB shared metadata ceiling; a larger cache explicitly fails this local profile. Keep producers excluded until the complete startup coordinator consumes all remaining proofs. The returned counters and empty observations do not alone establish readiness or certify pre-journal mutation provenance.

### 1.6 Executable M1 gateway composition

The opt-in `stepfunctions` binary feature registers the real typed HTTP target-family router before all heuristic path/signing-scope routers and the S3 catch-all. `SERVICES=stepfunctions` also works without the Lambda feature/service. Conflicting paths, duplicate targets and query-protocol content/signing headers still reach Step Functions envelope validation; no fallback into another service is allowed. It uses the validated default region/account, partition `aws`, and `STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION` (default true, the existing local-development policy). Strict mode requires the existing static credential provider before listening. Lambda Tasks are enabled only when the actual registered Lambda provider has an explicitly selected non-disabled executor and its scope matches. Otherwise compilation rejects Lambda Tasks while pure workflows remain available. No implicit code-execution selection is introduced. Shared gateway routing must pass strict production boundary lints: consolidate repeated SigV4 service classification into a borrowed iterator parser, and remove unchecked indexing/allocation from Lambda date-prefix classification while preserving existing route matches. Exercise malformed credentials and supported/rejected Lambda prefixes; this is gateway integration boundary closure, with no authentication-policy change.

Implement the M1-supported budgets from §1.2 in the existing root `budgets` mapping: workflowApiAccepted, workflowApiWorkers, workflowPaginationBytes, workflowPaginationTokens, workflowRegistryMachines, workflowRegistryVersions, workflowRegistryRecords, workflowRegistryCommands, workflowNameTombstones, workflowRetentionSeconds, workflowOpenExecutions, workflowEvaluations, workflowDispatches, workflowMaxDurationSeconds, workflowTransitions, workflowSnapshotBytes, workflowHistoryBytes, workflowTotalHistoryBytes, workflowLambdaAcceptedCalls, workflowLambdaCleanupSeconds and workflowDrainSeconds. Validate their ranges/relationships before allocation; execution history still has the static25,000-event ceiling. Frame and intermediate-result settings workflowFrames, workflowFramesPerExecution, workflowResultBytes and workflowResultBytesPerExecution are enforced by the structured supervisor. Callback budgets are installed by§1.7; the separate completed-record budget remains proposed until persistence/retention work installs it. No ignored configuration field may pretend to enforce a quota.

```text
validated YAML/scope ──► main composition ◄── actual registered Lambda provider
                              │                    (optional, explicit executor)
                              ▼
gateway: exact SFN target first → bounded typed HTTP → retained API/registry
                                                        │ captured revision
                                                        ▼
                                                execution supervisor
                                                        │ actual Task port only
                                                        ▼
                                                Lambda bridge → provider
```

The gateway capability response reports the installed JSONPath/eight-state profile, Inline Map concurrency ceiling40 and enforced frame/result quotas, whether the actual Lambda port is installed, effective local limits, and resources-and-history snapshot coverage under§3.3 with snapshotResume=false and paginationEpoch=fresh-on-restart. Available integration is distinct from final Docker qualification. The Step Functions transport owns its full modeled request deadline; the gateway must not race it with an earlier generic504 timer. Actual socket slow-body timeout qualification must assert the AWS JSON error and request-ID header. Readiness reads API state, registry worker/cleanup status, installed callback root status and installed bridge status without submitting a business command. Registry status is updated on command/maintenance boundaries (at most a one-second maintenance interval), and root loss closes readiness immediately. Last ordinary Lambda diagnostics (missing function, business failure, handler timeout or Stop) do not poison readiness after cleanup settles; permanent worker failure/uncertainty does. Manual Docker provider injection requires image_available readiness; the ordinary CLI still rejects unsupported Docker selection before listening.

### 1.7 Executable callback service composition

Compose the registered callback owner§5.5/D66 and public adapter§1.4/D67 in the real WorkflowRuntime when the actual selected Lambda port exists. Start one scoped CallbackRegistry/unique CallbackSupervisor, install its handle into the fresh ExecutionSupervisor, then compose Registry and API from that same binding. Pure services and disabled/unavailable/unqualified Lambda providers allocate no callback root and keep callback definitions/API unavailable. Preserve feature-minimal builds and explicit executor selection; this does not enable Docker startup or qualify its network.

Support YAML workflowCallbackTokens=4096 (1..65,536), workflowCallbackTombstones=100000 (1..1,000,000 and≥tokens), workflowCallbackCommands=1024 (1..100,000), workflowCallbackWorkers=8 (1..128). Build CallbackLimits with workflowRetentionSeconds (60..7,776,000s). Validate all fields and relationships before allocating/spawning roots, including pure-service configurations; reject unknown/invalid settings. Tombstone capacity includes active classification reservations, not an extra independent live+closed pool. Actual callback preparation/result bytes share the existing global/local workflowResultBytes budgets; do not add unenforced storage settings or make budget exhaustion a wait holding dispatch capacity.

WorkflowStatus retains a read-only handle to that exact callback root. Admission-independent readiness includes its status.ready in addition to API, registry and actual bridge health. Publish callbacks=true only when the real registered callback/SendTask composition is installed; this is integration availability, separately reported from dockerQualified=false and the independently qualified snapshot=resources-and-history contract under§3.3. Report all four effective callback limits and their shared retention interval. Root loss/close immediately makes readiness false without requiring a command slot; temporary live/retained/command saturation is an action quota error, not permanent worker unhealthiness. A missing Lambda port leaves callbacks=false.

Retain the callback controller in the unique WorkflowSupervisor. Normal resumable shutdown order is API close/join → original grace cutoff → registry cancellation/physical cleanup join → callback root close/join → bridge close/join → Lambda provider. A Callback phase is retained after observer expiry; all joins share the original global absolute deadline and never restart grace/mint/preparation. Preserve the first typed diagnostic if a later owner fails. On final-cleanup observation failure, signal registry, callback and bridge close independently before main attempts provider destruction; no successful consistency cut is claimed until all owners prove cleanup. Closing callback intake early must not certify execution/physical cleanup. A dropped unique controller still closes its own root through existing Drop behavior.

```text
validated YAML + explicit selected Lambda provider
              │
              ▼
       actual Lambda bridge ◄──── execution physical calls
              │
              └─ CallbackRegistry(scope, limits) + unique controller
                         │ install before any execution
                         ▼
                 ExecutionSupervisor → Registry → typed API
                         │               │          │
                         │ retained ASL  │ binding  └─ SendTask commands
                         └───────────────┴────────────▶ callback actor

readiness = API ∧ registry ∧ installed callback root ∧ installed bridge
shutdown  = API → grace → registry → callbacks → bridge → provider
expiry    = retain same phase/joins; final close signals every retained owner
```

Qualification: validate exact YAML boundaries/defaults/relationships and reported limits; pure/minimal services reject callbacks; actual Native composition accepts callback ASL and all three APIs; callback root closure bypasses saturated ordinary admission and poisons readiness; resumable/expired final shutdown retains phase and signals every owner. Before publishing this capability, the consuming binary must pass the real handler SDK acceptance below, including signed callbacks from Rust SDK and boto3 and consumed PID retirement. No dependency package/version changes or relaxed authentication are required for the feature.

## 2. Deployment topology and readiness

| Topology | Control path | Handler SDK path | Required test |
|---|---|---|---|
| Host Rustack + Linux Docker | Local socket + attach | Configured host-gateway address; bind gateway on an operator-approved reachable interface | Linux amd64/arm64 |
| Host Rustack + Docker Desktop | Desktop socket + attach | host.docker.internal with reachable gateway bind | macOS arm64 |
| Rustack server container + sibling functions | Mounted daemon socket + attach | Shared user bridge and stable Rustack service alias | Linux container lane |

The host example requires a suitable gateway listener; the default host loopback listener is not universally reachable from containers. Do not change the default listener merely because Docker is enabled. Startup diagnostics explain the chosen endpoint and its probe status. With no function-visible endpoint, pure handlers may run, but capabilities state that SDK callbacks are unconfigured. rootless Docker requires its limit capabilities to be probed; unsupported resource enforcement fails explicit Docker readiness rather than removing limits.

Expose capability fields for workflow type/query profile/state types/integrations, local quotas, snapshot coverage, Docker platform, backend probe result, relay protocol version and limitations. Distinguish available adapter from successfully executed test fixture. Health endpoints remain free of ordinary business admission starvation. Metrics contain bounded error classes/counts/durations, not function names, input, image credentials or task tokens as unbounded labels.

## 3. Shutdown, cancellation and snapshots

```text
┌─ admission boundary ───────────────────────────────────────────────┐
│ mark draining; reject new external work                           │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ producers and workflow workers ───────────────────────────────────┐
│ Events/SNS quiesce → Step Functions drain/cancel → Lambda quiesce  │
│                   owned children / tokens / invocations cleaned   │
│ DDB and other existing workers retain their required dependencies │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼ SQS/final downstream quiescence
┌─ consistency cut ──────────────────────────────────────────────────┐
│ no RUNNING execution; no unresolved container cleanup             │
│ success → existing atomic snapshot save → workers shutdown        │
│ failure → retain prior snapshot; diagnostics; nonzero exit         │
└───────────────────────────────────────────────────────────────────┘
```

The binary keeps the unique composite WorkflowSupervisor outside the shared RuntimeWorkers health view. It directly retains ApiSupervisor, RegistrySupervisor, optional CallbackSupervisor and optional LambdaBridgeSupervisor; no Mutex or additional unowned task is needed. Cancellation of an observing quiesce future retains those exact controllers and its shutdown phase. Final cleanup uses a separate shutdown entry point: skip grace, retain the same API/registry/callback/bridge joins, and synchronously signal all remaining controllers if the observer deadline expires before joining. A resumed final observer does not re-enter grace; uncertainty still forbids a successful consistency cut. One drain cutoff is captured on first quiesce; a resumed waiter never restarts the grace interval.

```text
main (unique mutable ownership)        gateway (shared read-only handles)
  WorkflowSupervisor                    WorkflowStatus
    ├─ ApiSupervisor     ◄────────────    API readiness
    ├─ RegistrySupervisor◄────────────    registry status watch
    ├─ CallbackSupervisor ◄────────────    callback root readiness
    └─ LambdaBridgeSupervisor             actual port readiness
       │
       ▼ one resumable shutdown phase
  API join → grace cutoff → registry cancel/join → callback join → bridge join
  final stop: skip grace; expiry → close retained owners, no successful cut
       │ clean only
       ▼
  Lambda quiesce → snapshot decision → final worker shutdown
```

Producer quiescence precedes the composite. Failure of HTTP drain or producer quiescence still enters final workflow cleanup before provider destruction. Cleanup failure is propagated, and the other providers still receive their final cleanup attempt under the global deadline. Dropping the composite closes all child controllers; their existing retained roots remain responsible for accepted work. Workflow resources/history are not added to the current snapshot format by M1 registration; capabilities and startup coverage must state this explicitly.

The workflow drain interval permits already accepted executions to finish. Then cancel remaining executions with ABORTED and error=Rustack.Shutdown, preserving terminal history. This is deliberate local shutdown behavior, not cloud-style pause/resume. Revoke callback tokens, cancel owned children and await Lambda cleanup before Lambda itself quiesces. Quarantine or unfinished cleanup prevents publishing a successful consistency snapshot. Quiescence must be idempotent and share one global absolute deadline, not restart a full timeout per worker.

The host and pinned relay image must implement control protocol version2 together (§3.8 of the runtime design). An older version1 relay is incompatible and fails initialization before function allocation. Rebuild/re-pin the relay on protocol upgrades; do not silently omit handler-timeout enforcement to accept an older image. Caller absolute budgets include queue/startup/execution; the captured function handler budget arms only at consumed Runtime API delivery. Cleanup remains independently owned if the caller budget expires.

LogType=Tail uses the invocation's bounded observed log window; environment-wide diagnostic tails remain separate. Window boundaries follow actor-observed completed chunks across independent stdout/stderr and control streams. Buffered/background output can arrive in a later window; this local runtime does not claim a causal producer flush barrier. A silent invocation never copies the prior collected diagnostic tail. Lost capabilities or unconsumed/expired mutation receipts invalidate capture and require environment retirement. No flush sleeps or extra Docker log replay are used.

The real binary API adapter uses [API§1.3/§6.1](./rustack-workflows-10-api-design.md#13-retained-real-api-adapter-and-preparation) the supported validated YAML limits from §1.6. Its root owns request workers and PageStore; token entries are count-bounded and share a separately byte-charged read capture. Charge vector capacity, not filtered length. Stop API admission and join its same workers before registry shutdown. Loss of API/Registry command handles alone does not cancel accepted executions while the unique registry supervisor remains alive. API health distinguishes a closed intake or retained worker failure, with typed diagnostics preserved through shutdown.

Registry constructor limits implement [API§3.1](./rustack-workflows-10-api-design.md#31-executable-registry-ownership-and-resource-lifecycle); §1.6 wires its supported YAML fields. One retained blocking worker acquires secure IDs while its pending command stays owned above the await. Source fingerprints/compilation are prepared by the embedding caller's bounded CPU owner. Record and name TTLs never release uncertain cleanup; deletion may remove mutable tags while the captured revision/resource identity remains retained. Immutable tag versions have independent allocation accounting, and cloned list snapshots share those allocations. Close the registry and observe the same joined shutdown, then close/join the installed callback root before closing its Lambda bridge. Root failure synchronously signals the execution owner and preserves the bounded graph instead of inferring cleanup.

The Step Functions Lambda bridge closes and joins before provider quiescence. Its supervisor retains one manager join across deadline expiry or a dropped shutdown waiter; unresolved Lambda handles and bridge slots remain charged, and the same cleanup observer can later settle the receipt without a second business outcome. See [integrations§1.1](./rustack-workflows-40-integrations-design.md#11-owned-admission-receipts-and-the-binary-lambda-bridge). The main composition supplies these validated budgets from the supported YAML fields in §1.6.

Within Lambda, stop deployment/invocation producers before retiring the pool/environment consumers. Join EnvironmentPool before EnvironmentManager, then join EnvironmentManager before revoking image leases, and join image cleanup before DockerWire shutdown. Environment count and aggregate function+relay memory remain reserved through allocating, idle, busy and quarantined states; a timed-out shutdown waiter leaves that ownership intact. A removed environment's business failure is distinct from failed supervision: known protocol/handler errors may retire cleanly, while Worker failure remains cached even after physical recovery. The lower-level owner alone is not a startup readiness receipt; historical uncertain effects and daemon/network/resource probes remain separate required gates.

Snapshot coverage:

| Resource | Persist | Restore behavior |
|---|---|---|
| Machine definitions/revisions, role/config/tags | Yes | Validate, compile and stage all records before replacing live state |
| Terminal executions/history and captured machine revision | Yes, within retention/byte budgets | Preserve outcomes, causality, pagination source data and DescribeStateMachineForExecution |
| Name tombstones | Yes | Retain remaining90-day reservations; reject overflow rather than shorten retention |
| RUNNING records in imported/unexpected archives | No resumption | Convert to ABORTED/Rustack.ExecutionInterrupted before serving; never replay uncertain side effects |
| Callback tokens/live handles/pagination handles | No | Fresh runtime epoch invalidates old handles; no plaintext token archive |
| Lambda image content references/config | Yes | Validate same digest/content availability before image execution; no tag re-resolution; serialized metadata never grants a live PinnedImage |
| Lambda deployment readiness/update state | Yes | Apply [runtime §2.10](./rustack-workflows-20-lambda-runtime-design.md#210-deployment-state-and-atomic-settlement): reject contradictory legacy/typed status; abandoned initial/update work becomes failure, retaining prior update code |
| Docker image layers, containers, tmpfs and code-process state | No | Images must be preloaded/allowed pulls must fetch exact digest; environments start cold |

Use the existing versioned service archives: `services/stepfunctions/meta.ss.zst` and `data.ss.zst` for definitions/indices and bounded history payloads. Store durable DTOs, not Arc/compiler heap/channel layouts. Recompile definitions with an archived profile version. Unknown profile versions or invalid references fail import atomically. Add versioned Lambda image metadata to its existing archive, preserving the existing artifact protections. Imported run records do not authorize deletion of containers from another owner.

Reserve terminal-record and name-tombstone capacity at Start admission so accepting a run never depends on finding retention capacity after its side effects finish. Reserve the terminal history allowance independently of ordinary event append capacity. History deletion cannot release a still-live name reservation.

There is no WAL: SIGKILL may lose all requests since the previous completed snapshot. A running-record conversion can explain an imported historical record; it does not recover executions that were never saved. Old completed snapshots remain recoverable under the existing replacement protocol. Restored pagination handles are invalid, and this epoch boundary is advertised.

### 3.1 Sealed registry ownership and durable export

Before workflow import, retain the actual registry state across its successful shutdown and implement the complete library export boundary. `Registry::export_snapshot(SnapshotLimits)` returns an owned, redacted-Debug, serde-Serialize `RegistrySnapshot` only after the unique registry root has drained every execution and proved physical cleanup. An accepting, merely closing, failed, panicked or quarantined root is not a sealed snapshot; an observer timeout while drain is still pending cannot expose one. The snapshot API does not use ordinary command admission and never starts/cancels work. The main snapshot coverage stays unsupported until the consuming restore/archive service is installed and qualified.

The root retains its state above every await as today. On successful drain, perform final maintenance, validate that every retained record has one terminal outcome, stop date and Clean cleanup and that every name claim has a terminal close observation. Move resource maps, immutable terminal views and name reservations into one immutable capture published through a watch slot shared with Registry. Retain the prepared capture in the unique owner's failure guard while destroying the drained runtime/port owner before publishing: a destructor panic must expose no cut and preserve the original allocations in bounded quarantine. Publish at most once before root success; do not serialize/clone all payloads inside the actor. The capture retains the same revision/tag/record/history allocations and permits through the last shared observer, including after a shutdown controller is dropped. Root failure publishes no capture and preserves the existing quarantined graph. This is the registry's consistency cut only; the main supervisor still must join callbacks/bridge/providers before global snapshot publication.

```text
Registry commands / child submissions → unique registry root
                                            │ close; original drain owner
                                            ▼
                           executions settle + physical cleanup joins
                                            │ final maintenance / invariants
                     ┌──────────────────────▼────────────────────────┐
                     │ immutable sealed capture (one watch slot)    │
                     │ moved resources + exact terminal read views  │
                     │ original allocation permits + name claims   │
                     └──────────────────────┬────────────────────────┘
                                            │ retained after root join
                         bounded caller CPU ▼
                     export_snapshot(SnapshotLimits) → durable DTO
                         error: retain same cut; no dispatch or replay
                                            │ later consuming service
                                            ▼
                         CBOR/archive boundary§4.2 → atomic save

root failure/quarantine → no sealed capture; no successful snapshot cut
main shutdown still joins callbacks → bridge → Lambda before global publish
```

Capture one adjacent paired Tokio-monotonic/UTC sample before final maintenance and capture traversal. Perform final maintenance relative to that same monotonic cut; never pair an early monotonic reading with UTC observed after a long traversal. Retained record expiry is close-observation+configured retention; name expiry is close-observation+90days. Project the remaining monotonic interval onto that sample's UTC without millisecond truncation, so restart downtime ages the remaining reservation and rounding never shortens it. Expired clean records/names are removed by final maintenance; uncertain cleanup cannot be archived. Freeze the timestamps once: repeated exports from the same capture are identical even when observed later. Preserve existing separate record/name TTL semantics, deletion/recreation identities and independent caller snapshot allocations.

Schema/profile versions start at1. Profile1 retains the existing effective STANDARD/JSONPath/OFF/tracing-disabled service contract; no unsupported mutable settings are invented. All DTO field names use camelCase; rows have deterministic order. The export is data only:

| Root / row | Durable fields |
|---|---|
| Root | schemaVersion, profileVersion, scope(partition/region/account), stateSupport(sequential/structured), lambdaSupport, callbackSupport, capturedAt, resources, revisions, machines, executions, names |
| resources (resourceId order) | resourceId, arn, createdAt; include identities captured by executions even after deletion/recreation |
| revisions (revisionId order) | revisionId, resourceId, definition(exact UTF-8 source), sourceHash(32 bytes), roleArn, publishedAt; include old captured revisions once |
| machines (ARN order) | resourceId, revisionId, status(active/deleting), updatedAt, tags |
| executions (execution ARN order) | arn, resourceId, revisionId, input(exact original JSON text), optional traceHeader, startedAt, stoppedAt, expiresAt, outcome, history |
| outcome (status tag) | SUCCEEDED + exact output text; FAILED or ABORTED + separately omitted/empty error/cause; TIMED_OUT without invented failure fields |
| history | rows {timestamp, eventJson} in ascending event ID order: timestamp is the exact ordinary UTC datetime; eventJson is the complete official JSON DTO string, including its public numeric timestamp and all payload text/causal fields |
| names (execution ARN order) | arn, expiresAt; include reservations whose retained execution has already expired |

HistoryEvent's generated epoch_seconds adapter is JSON-only and truncates to milliseconds; never serialize that DTO directly as CBOR. The separate ordinary timestamp retains nanoseconds. A future importer must validate eventJson as the official DTO, cross-check its numeric timestamp against the public projection of the exact timestamp, then restore the exact UTC instant; a disagreement rejects the archive. Preserve all other official fields byte-for-byte through the JSON string and keep injected-token projections intact. Qualify an actual Ciborium wrapper round trip with a nanosecond timestamp and demonstrate the direct-DTO CBOR failure/precision loss as a negative control.

Serialize no compiled heap layout, actor/channel/permit, monotonic Instant, task token/hash/epoch, pagination handle, child runtime lineage, live deadline, port, infrastructure diagnostic/source error or cleanup handle. Persist the installed instruction/Lambda/callback profile, not an inferred capability from definition text. Root `Debug` and errors expose fixed classes, not rows. Intentional definition/input/output/history data remain available to the authorized archive consumer; injected callback credentials stay absent through the already qualified history provenance boundary.

SnapshotLimits is a validated library source-copy budget of1byte..1GiB, default512MiB. Before cloning source/history data, conservatively charge4096 bytes of root/metadata allowance per resource/revision/machine/execution,512 per name, all exact definition/role/tag/input/output/error/cause/trace text bytes, and history's complete charged_bytes (which includes its fixed and terminal reserve allowances). Deduplicate shared resource/revision identities using borrowed keys before copying. Reject checked-arithmetic overflow, inconsistent UUID bindings or a budget overrun without returning a partial DTO or releasing the sealed source. History extraction must use bounded direct traversal, preserving every event rather than an API page; reserve its vectors fallibly. The embedding encoder independently enforces actual encoded section/archive bytes. This budget is an explicit bound on export work, not a claim that serialized bytes or total RSS equal it. Async embeddings must call synchronous export/serialization under their retained bounded CPU owner; no detached background encoder is introduced here.

Qualification: live/closing/saturated-command roots reject export; clean shutdown (including controller loss) publishes exactly one deterministic capture; timed-out/dropped observers resume the same root and cannot expose premature state; failed/quarantined cleanup never seals. Exercise deleted/recreated identities, captured old revisions/roles/raw input, tags, all terminal outcomes, causal/full histories beyond page boundaries, independent name/record expiry with fractional intervals, conservative budget failures/retry and redacted Debug. An actual Native workflow must export computed terminal output and original history after joined cleanup. Source allocations stay charged through existing retained read handles and the sealed capture. This dependency does not implement restore or advertise the CLI archive service; those remain mandatory next tasks under5.2.

### 3.2 Startup-only atomic registry restore

Consume the exact schema1 export with strict data-only Deserialize DTOs, then compile/validate and reserve a complete replacement before one actor commit. The library APIs are `Registry::prepare_snapshot(RegistrySnapshot, SnapshotLimits) -> PreparedSnapshot` (synchronous caller-owned bounded CPU work) and `Registry::restore_snapshot(PreparedSnapshot) -> RegistryReceipt`; acknowledgment is RegistryResult::Restored. No HTTP action is added. Main archive registration remains a subsequent dependency. An embedding must decode under a byte/depth/node/collection/scalar bounded redacting adapter and join its preparation worker before shutdown; neither serde deserialization nor a parsed document alone grants domain validity.

Use an atomic startup phase Open → Preparing → Serving. Any domain-valid business admission attempt closes Open for restore; invalid wire/domain requests do not. Preparing admits exactly one owning lease and rejects business admission independently of command capacity. Failed preparation or dropping an unsubmitted prepared graph releases all temporary charges before reopening Open. The lease binds to the exact Registry instance, not just matching account/region. A successful actor commit transitions permanently to Serving; subsequent restore is rejected even after deleting all resources. Closing/root loss dominates every phase. Keep health non-ready while Preparing. A prepared graph lost after submission remains owned by its accepted envelope; an expired/dropped acknowledgment never replays the replacement.

```text
bounded storage decoder → unvalidated RegistrySnapshot
                                  │ startup Open lease (exact instance)
                                  ▼ Preparing; business admission unavailable
               caller's retained bounded CPU owner
               ├─ validate all shapes/IDs/scope/profile/references/history
               ├─ compile every revision and verify exact source hash
               └─ reserve existing revision/tag/record/history pools
                                  │ no invocations, frames, tokens or IO ports
                                  ▼ PreparedSnapshot (graph first, lease last)
                       same bounded registry command mailbox
                                  │ no parsing/compilation/IO in actor commit
                                  ▼
               exact instance + Preparing + root not closing
               swap resource/execution/name maps once → Serving → ack

prepare/drop/failure → release partial graph, unchanged namespace, reopen Open
closing/foreign lease/failed commit → no replacement or business dispatch
observer loss after acceptance → same owned envelope; never repeat commit
```

The caller-owned prepared graph uses the exact actor allocation/history pools, not a second unaccounted store. Allocate these immutable quota handles during registry admission and share them with the preparer; maps remain actor-owned. Preparation constructs immutable terminal handles with Clean cleanup, closed view channels, no runtime deadline and empty runtime lineage. It allocates no open-execution/dispatch/frame/callback capacity, starts no task and invokes no Lambda/child/endpoint. The actor only moves the fully prepared maps and performs ordinary maintenance. Root consistency/export guards remain in force. Require the target to supply the same or stronger installed capabilities; reject a weaker target, unknown schema/profile, foreign scope and callback-without-Lambda profile before compilation. Then compile every revision and validate its history kinds against the archive's declared source state/Lambda/callback profile, independently of stronger target capabilities. This guarantees re-export can compile all captured revisions using its recorded installed profile.

Keep strict DTO bounds before exposing deserialized fields: resources/revisions/machines≤4096, executions≤100000, names≤1000000, history≤25000 per row; definition≤1MiB; history eventJson≤2MiB; input/output≤262144bytes; resource/revision UUID strings≤36; ARN≤256 except role≤2048; trace≤256; tags≤50 with key/value≤128/256; errors/causes≤256/32768; scope strings use existing TenantScope bounds. Reject unknown fields, duplicate tags, overlong fields/collections and contradictory outcome fields, including extra fields on RUNNING and TIMED_OUT. Decode each required/optional instant from at most64bytes of canonical ordinary Chrono UTC text; reject excess fractional precision, noncanonical offsets/formatting and leap seconds instead of silently normalizing them. These are absolute decode ceilings; live configured quotas apply to the retained replacement after expiry pruning. SnapshotLimits applies the same conservative metadata/text/history estimate as export before compilation/copying; compute history charge with the engine's actual fixed/event/terminal-reserve sizes, without trusting a serialized charge counter. Deserialize errors remain the embedding adapter's fixed redacted Schema class; domain/prepare failures remain static RegistryError classes.

Validate every row, including expired data, before treating it as ignorable. Require canonical secure UUID identities, unique resource/revision IDs, one current machine per ARN/incarnation, correct revision→resource bindings and no dangling/unused resource or revision rows. Parse/check every machine/execution/role ARN against the trusted tenant. Execution ARN must agree with its captured resource ARN; each retained/expired execution has exactly one matching name row. Every finite name expiry, including name-only rows, must lie in(capturedAt,capturedAt+90days]. Terminal record expiry must satisfy the same interval and be no later than its name reservation. Validate these original intervals before pruning expired-at-restore data. Names may outlive all execution/resource rows. Reject a capturedAt later than the paired restore UTC cut rather than clamp away clock skew. Do not require wall timestamps to be monotonically ordered: system UTC can step while event IDs retain causality. All persisted UTC instants must be representable by the public non-leap timestamp profile.

For every history row, preflight bounded JSON before official-model deserialization. Require contiguous IDs1..N, previousEventId in0..id-1, ExecutionStarted only at ID1, the supported profile's event/detail discriminator shape and field bounds, no intermediate terminal event, and a final terminal event matching stoppedAt/outcome/output/error/cause exactly for terminal records. Reject AWS-model fields the selected runtime never emits (including assignments, task credentials, alias/version metadata), and irrelevant fields for the chosen detail kind. Check direct scheduled function ARNs with the compiler's same scoped grammar. Timeout/heartbeat fields, when present, are positive and at most99999999seconds; heartbeat exists only on callback scheduling and is strictly less than the explicit timeout or99999999 when omitted. Input/output truncation flags follow actual runtime projections, including true only for callback TaskSubmitted output. Cross-check the raw numeric timestamp lexeme against the official canonical public projection of the separate exact UTC timestamp, then restore the nanosecond instant. Reject mismatches and noncanonical timestamp numbers; preserve payload text and all remaining typed data. Validate initial input against the captured original input. Rebuild history with the original IDs/prior links under the existing global/per-record limits and terminal reservation; no API projection or page boundary may lose records. Infrastructure diagnostics are not restored.

Schema1 decoding additionally recognizes an unexpected outcome {status:RUNNING}, with absent stoppedAt/expiresAt, a nonterminal history and a matching name with absent expiresAt. Such rows never resume. After all raw validation/compilation, capture one adjacent monotonic/UTC restoration sample and convert them to ABORTED/Rustack.ExecutionInterrupted with a fixed local cause, a fresh terminal event and new configured record retention/90day name reservation. Reserve the terminal event before committing; overflow rejects import rather than truncating history. A null/absent expiry is otherwise invalid. Ordinary exports remain terminal-only. No token, callback epoch, live frame, parent relation, invocation or pagination state can be supplied through this schema.

Represent retention explicitly as Running, ClosedAt(close observation, original TTL), or Until(imported monotonic deadline). New completions use current configuration; imported terminal record/name deadlines preserve their remaining UTC-derived duration and age during preparation/queue delay. A changed retention setting does not shorten or reset them. Keep record and name expiry independent, and retain uncertain cleanup as before. Terminal restored records have no live deadline; child/callback parent admission rejects them explicitly. Expired rows are still validated/compiled but do not consume retained record/name/history slots; prune again at commit. This dependency does not authorize an unqualified Docker backend, image tag resolution or execution during startup.

Qualification must round-trip actual exported data through CBOR and prepare/commit/read/re-export, preserve raw inputs/source/roles/tags/recreated identities, all outcomes and nanosecond/full causal history, retain/prune independent TTLs under changed configuration, reject schema/hash/scope/profile/ID/reference/history/expiry/field/collection corruption without publishing partial state, and release all temporary permits/leases on failure/drop. Prove exact-instance and startup-only admission, business rejection during preparation, retained accepted commit after observer loss, and close-before-submit rejection. Imported RUNNING rows must append the interruption event with zero Lambda/child calls; terminal parents cannot mint callbacks/children. Exercise an actual Native restored workflow, preserving old output/history without replay, then executing a new request with computed output. Keep the full runtime/SDK/security gates and independent review. Actual main meta/data archives and startup/shutdown process restart acceptance remain required next.

### 3.3 Main workflow archives and startup/shutdown ownership

Install D71/D72 in the existing named snapshot service, without a parallel persistence format. Register exactly one stepfunctions service with SnapshotKind::Data whenever the actual WorkflowRuntime is built, including pure/minimal binaries. Its meta.ss.zst contains the v2 ServiceMeta envelope/state section and a data-only RegistryMetadata document. Its data.ss.zst uses the existing ServiceData/directory/CRC envelope and exactly one fixed history.cbor payload, even for an empty registry. The metadata is the schema1 RegistrySnapshot shape with empty history arrays; RegistryHistories is {schemaVersion:1, histories:[{arn, history:[{timestamp,eventJson}]}]}. Both parts retain strict serde bounds/redacted Debug. Library into_archive_parts/from_archive_parts move data without JSON intermediaries or compiler/actor serialization. Join requires schema1 history data, empty metadata histories and exactly one unique matching history row per execution; reject missing/duplicate/orphan payloads before prepare_snapshot. The final combined document must pass every D72 invariant. Definition/input/output/trace are intentional authorized metadata; injected callback credentials remain absent by the existing provenance contract.

Use budgets.workflowSnapshotBytes (camelCase YAML), default536870912, supported1048576..1073741824. This bounds the D71/D72 conservative source-copy budget and each actual encoded metadata/history CBOR payload, with the existing lower structural/collection/scalar limits still enforced. A save that exceeds either bound fails before global publication and retains the previous committed generation. Raw history.cbor reads are regular-file-only bounded streaming reads of at most configured bytes plus one detection byte. The v2 envelope has a separate uncompressed allowance of payloadLimit+1024bytes for its fixed headers/directory, and a conservative compressed allowance of min(existing2GiB archive ceiling, 2*uncompressedAllowance+1MiB), each with a single overflow-detection byte. Enforce these bounds before allocating full compressed/decompressed input, and the actual section payload ceiling before copying section bytes. Apply the workflow limits during recovery validation as well as service load/save validation and data unpacking; a typed budget failure refuses startup without classifying a possibly valid target generation as corrupt or replacing it with a previous generation. Structural/CRC failures retain the existing recovery behavior. Physical envelope validity selects the current generation but does not authorize deleting its previous completed backup before domain imports. Keep .previous on profile/hash/scope/quota/schema rejection; retire it only after all enabled services import successfully, or after the next prepared generation validates and syncs before publication. Never silently roll back a domain/policy failure. For this service require exactly one state section and exactly two data sections; cap its encoded data directory at1024bytes and deserialize exactly one entry named history.cbor before extracting any files. Reject missing/extra history payload files and oversized metadata before typed decoding. Qualify exact-boundary incompressible framing round trips, oversized valid-envelope rejection, unchanged recovery generations under a reduced configured budget, and profile/hash/scope rejection with both current/previous bytes intact followed by successful import and previous retirement. Reject nonregular input before open; on Unix use safe nonblocking/no-follow flags and post-open file metadata checks to close FIFO/symlink swap races ([nix OFlag](https://docs.rs/nix/latest/nix/fcntl/struct.OFlag.html), existing0.31.3 cohort). Keep the actual typed JoinError behind a classified redacting worker wrapper/explicit accessor; raw panic text never enters Display/Debug/source-chain diagnostics. Fresh clock fences after ready IO/transaction/lease joins and before/after listener bind prevent a ready future from bypassing its expired observation deadline. Fixed diagnostics must never include serde row contents. Reuse the existing nix cohort through one Unix-only CLI dependency edge; no package/codec/version or advisory/lint waiver is added.

One workflow archive actor owns a single accepted operation/reply slot and one retained blocking CPU join. SnapshotService load transfers an owned metadata buffer and staging path to this actor; save transfers its staging path. Receiver loss never cancels accepted work or starts a replacement. Keep the active envelope/permit and CPU JoinHandle in the unique owner across awaits. CPU preparation decodes both parts under the shared bounded CBOR profile, joins them and returns the fully charged PreparedSnapshot; the owner transfers it to the same Registry and observes its one receipt. Save CPU work exports only a sealed root, splits and encodes the parts; the owner writes history.cbor before returning metadata. Disk operations use an absolute budgets.requestSeconds IO deadline. Retain the actual asynchronous disk task/join past observation expiry until physical completion; do not discard Tokio filesystem futures that can still own blocking IO. CPU work is retained until joined and must recheck the original deadline before issuing Registry commit or starting a write. A fatal actor/CPU failure closes admission and remains sticky; retain unresolved CPU/envelope ownership on owner panic. Errors/drop release ordinary temporary data and quotas; a controller observation timeout keeps the same worker/join and can be resumed.

```text
build services → real workflow registry/API + archive actor + unique controllers
       │ register shared archive handle in RuntimeProviders
       ▼ named-snapshot lease
existing v2 meta/data readers → owned metadata + private staging directory
       ▼ one accepted archive envelope / quota slot
archive owner ── retained blocking join ──► bounded CBOR decode + parts join
       │                                  source compile + charged preparation
       ◄────────────────────────────────── PreparedSnapshot
       ▼ exact Registry receipt / one commit
all enabled services loaded successfully → bind gateway listener → serve

HTTP drain → producer quiescence → workflow API/grace/registry/callback/bridge join
       ▼ Lambda/downstream quiescence proves the global consistency cut
existing save workers → archive owner → sealed export/split/CBOR → history.cbor
       ▼ existing meta/data envelopes + manifest + atomic generation replacement
final cleanup: archive close/join → retained workflow controllers → provider shutdown
failure/timeout at any stage → join accepted archive work; no unproven save/replay
```

Workflow quiesce must leave archive admission available for the subsequent sealed save. Final shutdown closes/joins the archive actor before closing Registry, including startup failure or a lost save/load observer; it then attempts all existing controller/provider cleanup under the same global deadline. Keep this final archive close/join separately resumable from workflow quiesce. Main bounds lease/load/bind observation by budgets.shutdownSeconds and must clean up built services on startup failure before returning, and must never save a failed startup's partially imported graph. Startup import happens before opening a public listener; an error in any service prevents serving. Existing cross-service import mutability is not replaced with a new distributed transaction. Drain every started service load/save task on the first result error before removing shared staging or returning; do not abort sibling futures containing accepted workflow archive work. Main retains one SnapshotSession containing acquisition, loaded state and the same load/save transaction above all observing awaits. Repeated load/save observations never start another import/publication, and saving after failed startup is rejected. The session retains one SnapshotTask join above observing awaits; its owned task keeps the cloned provider registry and named lease while generic archive IO/codec/service work is running. Observer expiry/drop never aborts that task or its sibling joins. Final cleanup first resumes that same transaction, then joins the workflow archive actor before Registry/provider destruction. A task panic quarantines its bounded provider/lease graph. An accepted save may complete after its observer expires using its already proven cut; never issue a second save, and rely on the existing generation recovery protocol. Cancellation of the outer observation leaves the named lease, transaction and archive controller retained until final cleanup. A completed save only follows a proven workflow/bridge/provider consistency cut. On a failed startup/save/cleanup observation, quarantine the single named OS lease until process exit because legacy storage futures can have uncertain physical retirement; never release it for a competing runtime while such effects could finish. Keep lease acquisition itself in one retained LeaseAcquisition join above the startup timeout; final cleanup resumes it, and a lost unresolved owner quarantines its exact join so a late-acquired lock cannot escape ownership. OS process exit releases this bounded lock quarantine. Successful runs retain/release the lease normally.

Advertise snapshot=resources-and-history in installed workflow capabilities/coverage only after this consumer is qualified. Add snapshotResume=false and paginationEpoch=fresh-on-restart; all old callback/page handles stay invalid because new roots/tables are constructed on each process start. Persistence capability does not imply Docker qualification, image availability, WAL/crash recovery of unsaved work, cloud pause/resume or live code-process restoration. Missing stepfunctions entries in older completed manifests start an empty workflow registry; unknown schema/profile, malformed parts, quota failure or missing registered payload rejects startup. A disabled service's manifest entry follows existing disabled-service behavior.

Qualification includes real v2 meta/data/manifest generations, empty/full round trips, independent TTLs and full history, malformed/missing/orphan/cross-part/schema/profile/hash/quota rejection with no partial workflow publication, redacted errors, retained CPU/receipt/drop/deadline ownership and failure-before-save preservation of the previous generation. Main full/minimal feature and routing/capability tests must consume the actual registered service. An actual strict-auth named-snapshot binary must run pure and Native Lambda work, gracefully save/restart, read old exact results/history/captured revisions/tags/name reservations, reject old pagination/callback handles, then execute a new real Lambda request without replay. Verify shutdown cancellation of live work is archived terminal only, SIGKILL loses unsaved work while the previous completed generation remains readable, and a corrupt workflow archive refuses to listen without changing that generation. Exercise process/PID cleanup and reuse existing SDK fixtures/Make targets. Performance/IaC/final image/platform release requirements remain separately required under5.3/5.4; do not claim M4 complete from this consumer alone.

## 4. Debugging and engineering contract

Diagnostics identify deployment Pending/Failed, backend unavailable, invalid image architecture, missing Runtime API polling, handler error, Task timeout, callback expiry, child failure and cleanup quarantine separately. Payload-bearing execution history is an intentional API capability; infrastructure logs are redacted. Operator log access remains a trust boundary.

Follow AGENTS.md Error/Async/Type/Safety/Serialization/Testing/Logging/Performance/Documentation: config errors identify key/source without secret values; resource ownership remains in actors; validation is repeated on archive import; decompression has byte/count/depth limits; shutdown tasks are joined; config and snapshot fixtures test rejection/atomicity; use existing archive mechanisms rather than per-record files. Authentication configuration reuses current gateway behavior; no new public Runtime API listener is created.

Explicit exception to AGENTS.md § Injection Prevention, URL/SSRF: an operator-configured local gateway may use HTTP and private/loopback addressing because local AWS emulation requires it. It is not an ASL-controlled outbound URL, and request Host headers cannot configure it. Function networking and Docker daemon registry networking remain separate operator trust boundaries. Local cancellation actively terminates a function container; this is resource hygiene, not a promise that AWS Lambda would stop code when a Step Functions Task times out. Already completed side effects survive both paths.

### 4.1 Patched compression cohort before workflow persistence

Close existing B06 before installing the workflow archive decoder. Use the current patched zstd0.14.3 / zstd-safe8.2.0 / zstd-sys2.1.2 cohort (or later compatible patched releases) across both the direct snapshot dependency and the transitive Squib loader graph. Keep the existing snapshot envelope/version/compression level, bounded streaming decode and independent archive CRC validation. Do not downgrade to a nominally unaffected old binding or waive advisory checks. The current [zstd release](https://docs.rs/zstd/0.14.3/zstd/) and local fresh RustSec database define the patched baseline.

The published squib-loader0.2.0 and upstream HEAD4f1fd673c20ab28328b47592138409ad68f12f76 still require zstd0.13. Apply one explicit crates.io path override under patches/squib-loader, excluded from Rustack workspace membership as an independently maintained third-party package. Preserve the published0.2.0 src/lib.rs byte for byte, normalized package metadata/dependency versions and API/type identity; change only its zstd requirement to the patched0.14 line. The rest of Squib stays on its pinned registry cohort. Include original manifest, VCS/package checksum, source fingerprints and upstream license notice so the exact overlay is inspectable. Preserve upstream's declared Apache-2.0 metadata and its separately supplied LICENSE.md notice verbatim; do not silently rewrite either. Do not copy its stale lockfile, edit Cargo's cache, add first-party unsafe/lint exceptions, or mutate/publish another repository. Retire the override when an upstream compatible release supplies the patched dependency. Source/binary build distributions must include the overlay directory and root manifest. Independently published Rust crates consumed under another workspace require that root to repeat the override or adopt the upstream fixed loader, because [Cargo applies patches only from the workspace root](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section). This dependency closure is verified for the current Rustack workspace.

Make qualification must verify the overlay source/manifest provenance, select the intended patched loader package, execute its existing synthetic kernel decompression/size/magic/overflow regressions and the runtime snapshot suite, and fail on any remaining vulnerable zstd cohort. Add a harmless pre-upgrade snapshot frame generated with the original resolved0.13.3 graph as compatibility evidence; the updated decoder must preserve its typed envelope/section bytes and reject truncated/corrupt input. Reusable raw encoder/decoder contexts also need a regression across an abandoned stream and subsequent frame, including bounded decoding, matching the repaired leak class. These are library/codec regressions, not new VM/Docker/platform qualification.

```text
snapshot archives ──► direct zstd ~0.14.3 ───────────────┐
                                                       ▼
squib0.2.0 → vmm → patched loader0.2.0 → zstd0.14 → safe8.2 → sys2.1.2
                         │
                         └─ original source/API + one dependency override

acceptance = source provenance + old-frame/kernel regressions
             + normal/strict workspace gates + fresh audit/deny
```

Fresh cargo audit must have no vulnerabilities or unsound compression warnings, and all cargo deny checks must pass without new ignores. Historical B01/B06 evidence remains chronological; mark B06 closed only after inspecting the current resolved graph and gates. Workflow archive export/import, frozen-registry lifetime, cross-service consistency and final image/platform acceptance remain separate work.

### 4.2 Bounded CBOR import boundary

Before adding workflow DTO import, replace the generic snapshot CBOR helpers with one shared preflight/encode boundary, used by manifest, service metadata and data-directory readers. A compressed/envelope byte cap alone does not bound a forged container's declared allocation or a decoder's recursion. Keep the v2 archive envelope, existing service DTO shapes and compression unchanged. This is an installed archive boundary prerequisite; it does not advertise workflow persistence.

The Rustack v2 CBOR profile permits unsigned/negative integers, definite byte/text strings, booleans/null/simple/floating values and definite or indefinite arrays/maps. Map keys are definite UTF-8 text or unsigned/negative integers; reject duplicate decoded keys even when their integer/length headers use different widths. Reject tags, indefinite byte/text strings, reserved header encodings, invalid UTF-8, misplaced breaks, odd indefinite maps, truncated values and any bytes following the single root value. These restrictions match the current serde-generated archive DTOs, including integer-keyed Lambda version maps, byte payloads and unknown-length serde collections; arbitrary RFC8949 data is not a Rustack archive schema. Preserve non-minimal integer/length encodings rather than require a new canonical encoder. Follow the [CBOR wire grammar](https://www.rfc-editor.org/rfc/rfc8949.html#section-3); the domain profile intentionally rejects unneeded general-purpose features.

| Structural budget | Ceiling | Enforcement |
|---|---:|---|
| One CBOR section | 2GiB | Before traversal, capped fallible writer while encoding |
| Container depth | 128 | Before descending; scalar root depth0, array/map root depth1 |
| Aggregate values (root, keys, values and containers) | 8,000,000 | Before visiting each item; includes ignored schema fields |
| One array / map | 1,000,000 elements / pairs | Before definite-container traversal; during indefinite traversal |
| One definite byte/text scalar | 128MiB | Before reading/decoding declared contents; accommodates existing base64 Lambda ZIP metadata |
| One text map key | 128MiB (same generic scalar ceiling) | Before UTF-8/key tracking; integer keys require no string allocation |

The generic text-key ceiling intentionally equals the scalar ceiling: existing service maps can contain user data, including DynamoDB attribute names above4KiB, and this prerequisite must not silently impose workflow identifier limits on them. Per-workflow schema validation must choose narrower domain bounds. The preflight borrows the input, builds no dynamic value tree and copies no text. Only the active maps retain bounded duplicate-key sets. Use checked arithmetic/conversions for every external count/length and compare minimum remaining value bytes before loops. Run the existing Ciborium typed decoder only after the entire structure passes, with an explicit256 [Ciborium recursion limit](https://docs.rs/ciborium/0.2.2/ciborium/de/fn.from_reader_with_recursion_limit.html) and exact reader consumption. The typed limit includes headroom for Ciborium's additional enum recursion entries: a scalar unit enum also enters its recursion guard, so it must exceed the128-container wire depth. Structural and schema failures expose fixed redacted error classes, with no attacker-controlled Ciborium diagnostic/source in the error chain. Schema-specific quotas/references, unknown-field policy, compiled definition validation and aggregate live/history accounting remain the workflow importer's responsibility; these generic ceilings do not claim to enforce them or guarantee a small total process RSS.

The writer must enforce the same byte/structural profile before returning an encoded section, so a successful save never produces a section the shared reader would reject. Allocation failure and oversize encoded output return errors; the existing atomic replacement protocol retains the previous completed snapshot. Do not change the existing cross-service import publication contract or perform blocking parser work in a newly detached task. Workflow preparation will own its bounded CPU/restore transaction separately.

```text
archive bytes → bounded zstd → v2 envelope / CRC → CBOR section
                                                  │
                           ┌──────────────────────▼─────────────────┐
                           │ complete borrowed preflight            │
                           │ bytes/depth/nodes/collections/scalars   │
                           │ UTF-8 / key uniqueness / exact root    │
                           └──────────────────────┬─────────────────┘
                                                  ▼ only on success
                                  typed Ciborium decoder (limit256)
                                                  ▼
                                  existing service import validation

service DTO → capped fallible writer → same preflight → archive publish
failure at either boundary → fixed redacted class; no new snapshot publication
```

Qualification must exercise real manifest/service/data-directory round trips, the original pre-upgrade frame, definite/indefinite collections, integer-keyed maps, actual DynamoDB export/decode/import with long attribute names, byte/float encodings and derived enum trees at the exact128-container boundary. Test every numeric header width, malformed/truncated inputs, invalid text, alternate-width duplicate keys, empty/root/trailing cases and every resource ceiling at/either side of its boundary using small test budgets. A Deserialize sentinel proves hostile structure is rejected before any typed visitor runs; an oversized serializer proves the writer stops while serializing. The actual SnapshotConfig save/load path must preserve and successfully reload its previously committed generation when an encoder rejects a duplicate-key section. Use bounded property cases for valid nested profile values and arbitrary hostile bytes; no panic, unchecked index or input-bearing diagnostics. Expose these tests through Makefile and retain standard/minimal-feature Rust gates. No dependency/version upgrade or Docker/VM work is required.

## 5. Cross-references

- ← [runtime hardening](./rustack-runtime-hardening-design.md), [binary archives](./ruststack-snapshot-binary-archive-design.md)
- ↔ [verification](./rustack-workflows-70-verification-plan.md), [implementation](./rustack-workflows-90-impl-plan.md), [decisions D8/D10](./rustack-workflows-99-decisions.md)

### Controlled pull library dependency

[Runtime§2.14](./rustack-workflows-20-lambda-runtime-design.md#214-controlled-if-missing-pulls-and-retained-progress-transport) supplies validated registry/credential policy and retained ifMissing preparation. Main must load the bounded private YAML credential file, construct these types and select start_with_pull explicitly; default start never pulls. Only initial missing-image lookup triggers network work. The daemon performs registry networking; exact allowlist matching is not a guarantee about its mirrors/DNS/redirects. Close/join preparation and wire work during shutdown, and never delete source cache references as runtime cleanup. YAML loading, endpoint/readiness and final startup acceptance remain required consuming work.
