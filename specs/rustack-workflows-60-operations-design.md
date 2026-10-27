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
| workflowHistoryBytes / workflowTotalHistoryBytes |32 MiB /256 MiB|1..256 MiB /8 MiB..4 GiB; per-execution≤total |
| workflowCompletedExecutions |10,000|1..100,000; do not evict unexpired retained records |
| workflowRetentionSeconds |86,400|60..7,776,000; terminal history retention |
| workflowNameTombstones |100,000|1..1,000,000; always retain names 90 days |
| workflowPaginationTokens |10,000|1..100,000; token TTL 24h |
| workflowCallbackTokens / workflowCallbackTombstones |4,096 /100,000|1..65,536 /1..1,000,000; retained active+closed entries≥live tokens; closed-token TTL follows workflowRetentionSeconds |
| workflowCallbackCommands / workflowCallbackWorkers |1,024 /8|1..100,000 /1..128; commands include accepted actions and ready acknowledgments; workers bound retained mint entropy |
| lambdaDockerEnvironments |32|1..32 initially, including busy/idle/quarantined environments |
| lambdaDockerMemoryBytes |8 GiB|256 MiB..1 TiB; includes relay caps and all allocated environments |
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

Before ordinary admission, call reconcile_containers under the same DirectoryLease. It processes historical functions before their relays, leaves current-run records untouched and returns remaining/settled counts. Unknown404 remains unresolved; a positive exact identity is pinned durably before ID-only retirement. Repeating a cancelled/failed pass resumes the saved state. Foreign identity or corrupt/unsafe files fail closed. This operation covers recorded environment containers; private image aliases, other mutation classes, allowed pulls and the complete coordinator remain required before production readiness.

`make test-lambda-recovery` covers bounded/corrupt files, restart transactions, cancellation, foreign resources and delayed commits. `make test-lambda-recovery-docker WORKFLOWS_DOCKER_SOCKET=<socket-path> WORKFLOWS_ARCH=arm64` provisions actual owned containers, releases/reacquires the directory under a new run and retires the historical pair. Failed actual qualification preserves durable ownership evidence rather than deleting the temporary data root.

### 1.6 Executable M1 gateway composition

The opt-in `stepfunctions` binary feature registers the real typed HTTP target-family router before all heuristic path/signing-scope routers and the S3 catch-all. `SERVICES=stepfunctions` also works without the Lambda feature/service. Conflicting paths, duplicate targets and query-protocol content/signing headers still reach Step Functions envelope validation; no fallback into another service is allowed. It uses the validated default region/account, partition `aws`, and `STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION` (default true, the existing local-development policy). Strict mode requires the existing static credential provider before listening. Lambda Tasks are enabled only when the actual registered Lambda provider has an explicitly selected non-disabled executor and its scope matches. Otherwise compilation rejects Lambda Tasks while pure workflows remain available. No implicit code-execution selection is introduced. Shared gateway routing must pass strict production boundary lints: consolidate repeated SigV4 service classification into a borrowed iterator parser, and remove unchecked indexing/allocation from Lambda date-prefix classification while preserving existing route matches. Exercise malformed credentials and supported/rejected Lambda prefixes; this is gateway integration boundary closure, with no authentication-policy change.

Implement the M1-supported budgets from §1.2 in the existing root `budgets` mapping: workflowApiAccepted, workflowApiWorkers, workflowPaginationBytes, workflowPaginationTokens, workflowRegistryMachines, workflowRegistryVersions, workflowRegistryRecords, workflowRegistryCommands, workflowNameTombstones, workflowRetentionSeconds, workflowOpenExecutions, workflowEvaluations, workflowDispatches, workflowMaxDurationSeconds, workflowTransitions, workflowHistoryBytes, workflowTotalHistoryBytes, workflowLambdaAcceptedCalls, workflowLambdaCleanupSeconds and workflowDrainSeconds. Validate their ranges/relationships before allocation; execution history still has the static25,000-event ceiling. Frame and intermediate-result settings workflowFrames, workflowFramesPerExecution, workflowResultBytes and workflowResultBytesPerExecution are enforced by the structured supervisor. Callback budgets are installed by§1.7; the separate completed-record budget remains proposed until persistence/retention work installs it. No ignored configuration field may pretend to enforce a quota.

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

The gateway capability response reports the installed JSONPath/eight-state profile, Inline Map concurrency ceiling40 and enforced frame/result quotas, whether the actual Lambda port is installed, effective local limits, and no workflow snapshot coverage. Available integration is distinct from final Docker qualification. The Step Functions transport owns its full modeled request deadline; the gateway must not race it with an earlier generic504 timer. Actual socket slow-body timeout qualification must assert the AWS JSON error and request-ID header. Readiness reads API state, registry worker/cleanup status, installed callback root status and installed bridge status without submitting a business command. Registry status is updated on command/maintenance boundaries (at most a one-second maintenance interval), and root loss closes readiness immediately. Last ordinary Lambda diagnostics (missing function, business failure, handler timeout or Stop) do not poison readiness after cleanup settles; permanent worker failure/uncertainty does. Manual Docker provider injection requires image_available readiness; the ordinary CLI still rejects unsupported Docker selection before listening.

### 1.7 Executable callback service composition

Compose the registered callback owner§5.5/D66 and public adapter§1.4/D67 in the real WorkflowRuntime when the actual selected Lambda port exists. Start one scoped CallbackRegistry/unique CallbackSupervisor, install its handle into the fresh ExecutionSupervisor, then compose Registry and API from that same binding. Pure services and disabled/unavailable/unqualified Lambda providers allocate no callback root and keep callback definitions/API unavailable. Preserve feature-minimal builds and explicit executor selection; this does not enable Docker startup or qualify its network.

Support YAML workflowCallbackTokens=4096 (1..65,536), workflowCallbackTombstones=100000 (1..1,000,000 and≥tokens), workflowCallbackCommands=1024 (1..100,000), workflowCallbackWorkers=8 (1..128). Build CallbackLimits with workflowRetentionSeconds (60..7,776,000s). Validate all fields and relationships before allocating/spawning roots, including pure-service configurations; reject unknown/invalid settings. Tombstone capacity includes active classification reservations, not an extra independent live+closed pool. Actual callback preparation/result bytes share the existing global/local workflowResultBytes budgets; do not add unenforced storage settings or make budget exhaustion a wait holding dispatch capacity.

WorkflowStatus retains a read-only handle to that exact callback root. Admission-independent readiness includes its status.ready in addition to API, registry and actual bridge health. Publish callbacks=true only when the real registered callback/SendTask composition is installed; this is integration availability, separately reported from dockerQualified=false and snapshot=unsupported. Report all four effective callback limits and their shared retention interval. Root loss/close immediately makes readiness false without requiring a command slot; temporary live/retained/command saturation is an action quota error, not permanent worker unhealthiness. A missing Lambda port leaves callbacks=false.

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

## 4. Debugging and engineering contract

Diagnostics identify deployment Pending/Failed, backend unavailable, invalid image architecture, missing Runtime API polling, handler error, Task timeout, callback expiry, child failure and cleanup quarantine separately. Payload-bearing execution history is an intentional API capability; infrastructure logs are redacted. Operator log access remains a trust boundary.

Follow AGENTS.md Error/Async/Type/Safety/Serialization/Testing/Logging/Performance/Documentation: config errors identify key/source without secret values; resource ownership remains in actors; validation is repeated on archive import; decompression has byte/count/depth limits; shutdown tasks are joined; config and snapshot fixtures test rejection/atomicity; use existing archive mechanisms rather than per-record files. Authentication configuration reuses current gateway behavior; no new public Runtime API listener is created.

Explicit exception to AGENTS.md § Injection Prevention, URL/SSRF: an operator-configured local gateway may use HTTP and private/loopback addressing because local AWS emulation requires it. It is not an ASL-controlled outbound URL, and request Host headers cannot configure it. Function networking and Docker daemon registry networking remain separate operator trust boundaries. Local cancellation actively terminates a function container; this is resource hygiene, not a promise that AWS Lambda would stop code when a Step Functions Task times out. Already completed side effects survive both paths.

## 5. Cross-references

- ← [runtime hardening](./rustack-runtime-hardening-design.md), [binary archives](./ruststack-snapshot-binary-archive-design.md)
- ↔ [verification](./rustack-workflows-70-verification-plan.md), [implementation](./rustack-workflows-90-impl-plan.md), [decisions D8/D10](./rustack-workflows-99-decisions.md)
