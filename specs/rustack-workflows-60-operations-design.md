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
| workflowOpenExecutions |128|1..4,096, including children |
| workflowDispatches |32|1..1,024; released while waiting on child/callback |
| workflowFrames / workflowFramesPerExecution |4,096 /512|1..65,536 /1..4,096; per-execution≤global |
| workflowMaxDurationSeconds |3,600|1..31,536,000; independent local ceiling even without ASL TimeoutSeconds |
| workflowTransitions |100,000|1..1,000,000; history may exhaust first |
| workflowHistoryBytes / workflowTotalHistoryBytes |32 MiB /256 MiB|1..256 MiB /8 MiB..4 GiB; per-execution≤total |
| workflowCompletedExecutions |10,000|1..100,000; do not evict unexpired retained records |
| workflowRetentionSeconds |86,400|60..7,776,000; terminal history retention |
| workflowNameTombstones |100,000|1..1,000,000; always retain names 90 days |
| workflowPaginationTokens |10,000|1..100,000; token TTL 24h |
| workflowCallbackTokens / workflowCallbackTombstones |4,096 /100,000|1..65,536 /1..1,000,000; closed-token TTL follows execution retention |
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

The workflow drain interval permits already accepted executions to finish. Then cancel remaining executions with ABORTED and error=Rustack.Shutdown, preserving terminal history. This is deliberate local shutdown behavior, not cloud-style pause/resume. Revoke callback tokens, cancel owned children and await Lambda cleanup before Lambda itself quiesces. Quarantine or unfinished cleanup prevents publishing a successful consistency snapshot. Quiescence must be idempotent and share one global absolute deadline, not restart a full timeout per worker.

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
