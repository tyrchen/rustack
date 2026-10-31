# Workflows 55: Server-in-Container Qualification

Status: Executable design; implementation and actual acceptance pending · Updated: 2026-10-31 · Work item: D98 /5.3e

## 1. Outcome and dependency

Run the actual registered Rustack Linux executable inside a Docker container, using the same daemon as its managed Lambda function/relay pairs. Ordinary signed host clients must create/invoke image functions and execute nested ASL; a real handler must call the containerized Rustack S3/DynamoDB gateway through its ordinary SDK. Readiness and graceful shutdown retain the existing production provenance, endpoint, kernel and physical cleanup requirements. This consumes [D97](./rustack-workflows-50-iac-design.md) and the existing [runtime](./rustack-workflows-20-lambda-runtime-design.md), [operator configuration](./rustack-workflows-60-operations-design.md#11-docker-configuration) and [verification](./rustack-workflows-70-verification-plan.md) contracts.

This is a qualification topology, with a purpose-built fixture image containing the actual CLI and an explicit lambda,stepfunctions,s3,dynamodb feature set. It does not certify the repository's default release Dockerfile, change default source features, introduce Docker-in-Docker or close the remaining native-x86/reference/Desktop/rootless/full-M4 requirements. No production executor, ASL or isolation relaxation is planned.

## 2. Topology and trust boundaries

```text
┌─ host test root: retained ServerContainer owner ─────────────────────────┐
│ private intent + env file │ bounded/joined Docker CLI │ SDK child owner │
│ client socket: operator-selected local Unix endpoint                    │
└──────────────┬───────────────────────────────┬────────────────────────────┘
               │ management                   │ signed host SDK
               ▼                              ▼ published loopback port
┌─ selected Linux daemon / explicitly qualified user bridge ─────────────┐
│ ┌─ Rustack server: immutable Linux image ID ──────────────────────────┐ │
│ │ /rustack, UID/GID65532; supplemental socket GID; root read-only       │ │
│ │ daemon-side socket → /run/rustack/docker.sock (server only)          │ │
│ │ exact owned named volume → /data; DATA_DIR=/data/state              │ │
│ │ ordinary gateway 0.0.0.0:4566; known unique server DNS name           │ │
│ └────────────┬─────────────────────────▲───────────────────────────────┘ │
│              │ existing DockerWire     │ default-provider S3/DDB SDK    │
│              ▼                        │ http://<server-name>:4566/     │
│ ┌─ normal managed Lambda environment ─┴──────────────────────────────┐ │
│ │ relay + function, shared network namespace only; existing limits    │ │
│ │ no server socket, data volume, host config mount or published port  │ │
│ └────────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

Distinguish three independent addresses: the host client's Unix socket; the socket bind source on the daemon's Linux filesystem; and the server DNS origin used by relay/function traffic. Docker [bind mounts use daemon-host paths](https://docs.docker.com/engine/storage/bind-mounts/), so a macOS client socket cannot be inferred to be a mountable VM path. A [user-defined bridge supplies container-name DNS](https://docs.docker.com/engine/network/drivers/bridge/); qualify that actual path through the production startup probe and handler SDK, without a guessed container IP or host-network fallback. Only the external SDK uses the independently observed loopback-published port.

## 3. Explicit inputs and bounded artifacts

Add WORKFLOWS_SERVER_IMAGE, WORKFLOWS_DOCKER_DAEMON_SOCKET and WORKFLOWS_DOCKER_DAEMON_SOCKET_GID to Make's discoverable inputs and pass them through the test environment. The existing client socket, selected/other daemon, architecture, bridge and image builder inputs remain required. The daemon-side socket and GID have no inferred client-path default; missing values fail the required gate before allocation. For the retained development VMs, read-only preflight observes /var/run/docker.sock, root:GID991, mode0660 on both; this observation is not portable configuration or native-x86 evidence.

Validate the Linux bind source before CLI dispatch: absolute ASCII path,1..256 bytes, only letters/digits/underscore/hyphen/dot/slash, no empty/dot/parent components, NUL, commas, whitespace or trailing slash. GID is canonical decimal0..2147483647. Resolve the existing user bridge to its exact qualified full ID. Image references use the existing validated grammar; capture its nonempty Linux/selected-architecture content ID and use that ID with --pull=never. Names and labels derive only from one fresh UUID: server name rustack-server-<32 lowercase hex>, volume rustack-server-data-<same hex>, one exact task nonce and distinct server/server-data roles. Record the actual daemon UUID and source/network IDs before effects.

Build the real CLI with the pinned Rust toolchain and existing Cargo Lambda/GNU target path, --locked --release --no-default-features --features lambda,stepfunctions,s3,dynamodb. Reuse the already pinned provided.al2023 fixture base, whose libc supports these Linux artifacts; do not place a GNU-linked executable in scratch. The image replaces the Lambda entrypoint with /rustack, contains a fixed public YAML budget file and initializes /data ownership to65532:65532. It contains no credentials, operator config, socket, host executable or host data. The Make build context is the selected architecture's bounded fixture output, not the repository or node_modules. Capture source ID and architecture after build.

The test owner creates its private host evidence root before allocation, mode0700, and an exclusive mode0600 env file consumed locally by Docker --env-file. No env-file bind mount is used. It sets only the explicit four services, DATA_DIR, fixed YAML path, strict service authentication, known local test authentication/SDK pair, Docker settings and the unique DNS gateway origin. Clear ambient AWS selectors through the existing qualified SDK policy. Keep raw credentials/env bodies out of diagnostics, report fields and command argv; retain the private env file only as owned evidence when cleanup is uncertain.

## 4. Server policy and readiness

The server uses UID/GID65532 plus the explicit supplemental socket GID, --cap-drop=ALL, no-new-privileges, read-only root,512MiB memory with equal memory+swap, one CPU,128 PIDs,64MiB noexec/nosuid/nodev temporary storage and a json-file log driver bounded to1MiB/one file. Its only writable persistent mount is the exact owned local named volume at /data; the only bind mount is the explicit socket, mounted read-only at /run/rustack/docker.sock. The image prepares that target path before the read-only root is used. No devices, host PID/network namespace, arbitrary bind, privileged mode, automatic removal or restart policy is allowed. A read-only socket mount still grants trusted server API control; it is never copied into a Lambda plan. Validate actual inspected user, groups, limits, security flags, mount types/sources/destinations, log limits, network ID and port bindings before accepting readiness.

The server's constant YAML keeps startup180s/shutdown60s and the existing60s Docker management/start/cleanup observer budgets, two environments/two dispatches and768MiB aggregate Lambda memory. Existing function/relay policies and init20s apply. Set LAMBDA_DOCKER_CROSS_ARCHITECTURE=false: the actual Linux server executable and managed images must match the selected daemon architecture. Outer evidence still identifies the amd64 VM as QEMU emulation. Publish only127.0.0.1:<daemon-selected-port>:4566; validate a unique nonzero port and use it for the host SDK. The injected gateway remains http://<server-name>:4566/, including its distinct internal port.

Require ordinary capabilities to report ready=true, Docker ready with the correct Linux platform and sdkGateway=true, and installed Step Functions integration support. A live PID or published port alone is not readiness. Startup's actual container endpoint/kernel probes must finish before ordinary ingress. Bound each health response by the existing helper and each observation by its original deadline.

## 5. Ownership, failure and shutdown

The retained test owner is separate from the existing host-process Server; a container must not masquerade as its Child. Reuse the unchanged Runner for direct Docker CLI processes and the existing Python PID barrier for the SDK child. Each command uses argv, explicit client socket, cleared environment, null stdin and independently drained32KiB stdout/stderr caps. Physically join every command, including timeout/forced termination, before interpreting its result. One600s original observation cutoff covers preparation, readiness and the SDK consumer; individual CLI calls are bounded by60s and cannot reset that cutoff. Image builds/provisioning occur before this workload. Existing SDK480s and IaC1020s cutoffs remain unchanged.

Persist intent before volume/container creation. Never retry an uncertain create or treat404/CLI death as proof that no daemon mutation occurred. Resolve known names only under the same engine, exact nonce/role labels, immutable source ID and expected volume; mismatched or unproven resources are retained, never adopted/deleted. Any forced/uncertain mutating command fails qualification and retains intent/evidence even if an immediate inventory is empty. All exercise errors flow through the same owner's cleanup, above the SDK future; assertions about the exercise occur only after cleanup results are collected.

After readiness, independently read only owner.v1 and daemon.v1 from the actual /data/state/.rustack-lambda-docker directory through bounded Docker cp-to-stdout archives, not host extraction. Verify the inspected server has exactly the expected named volume at /data. Each archive is≤32KiB, contains one expected regular basename with UID/GID65532, mode0600 and exact56/72-byte body. Allow at most one≤1KiB PAX timing header with≤4 bounded mtime/atime/ctime decimal values; reject other metadata, links, special/directory/extra entries, alternate paths, duplicate records, malformed/truncated checksums and nonzero data after the archive terminator. Use the existing workspace tar0.4.46 as a test dependency with raw-entry iteration; do not unpack. Require owner magic RSDKOWN2 and daemon magic RSDKDMD1, their SHA256 checksums, nonnil UUIDs, matching owner UUID and the actual engine UUID. These records bind subsequent independent complete runtime-container/private-image inventories to this server; a similarly named unrelated container cannot establish ownership.

On completion/error, send SIGTERM to the verified exact server container when still running and wait for its actual exit. A positive resource requires exit0; a negative startup requires its specified nonzero failure. Forced kill or cleanup uncertainty never qualifies. Use a separate180s cleanup cutoff, with at most70s for graceful server exit; remaining observations/removals share that original cleanup cutoff. Capture bounded logs privately and scan for credential leakage without printing candidates. Before removing the server or volume, require the production shutdown to have left zero owner runtime containers/private aliases/volumes, with no repair performed to turn failure into success. The positive SDK leaves its warm function alive so shutdown must retire a real function/relay pair.

Then re-inspect exact engine/container ID/name/nonce/source/mounts, remove only that stopped server, verify no remaining attachment to its exact named volume, revalidate volume name/local driver/labels and remove only that volume. Confirm both are absent and all captured operator source IDs unchanged. On unknown ownership, late effects, nonzero unexpected exit or unproven physical cleanup, fail and retain the directory/volume/known resources for diagnosis. No global prune, context change, VM restart or unrelated cleanup is allowed.

## 6. Independent positive and negative oracles

The positive ordinary signed boto3 consumer creates one real Node SDK image Lambda and S3/DynamoDB resources. Direct21 and23 calls return literal42/46 with warm counters1/2; independently read each exact object body and consistent DynamoDB result. A parent uses states:startExecution.sync:2 to await a child containing optimized Lambda invocation with25; require literal50/counter3, a new child execution ARN, the JSON Output distinction and causal successful histories. Validate actual SDK versions against the pinned3.1105.0 fixtures. Keep the exact function/relay IDs warm across calls; independently inspect authoritative environment injection to require the literal DNS origin, strict explicit local credentials, cleared conflicting selectors and no credentials in the relay. Inspect both managed containers to reject any server socket/data bind or published Runtime API. Delete workflow/data test resources but leave the warm function for server shutdown. Report only fixed cases, versions, counts and validated IDs/booleans.

The child has one optimized Lambda Task with Parameters.FunctionName fixed to that function, Payload.$=$, OutputPath=$.Payload, Task timeout60s and execution timeout90s. The parent has one states:startExecution.sync:2 Task with the fixed child ARN, Input.$=$, OutputPath=$.Output, Task timeout90s and execution timeout120s. Its third SDK event uses key=third, value25 and mode=roundtrip. The final parent/child JSON outputs must contain computed50/counter3, not an encoded JSON string; verify the parent Submitted history references the actual child ARN and the documented causal ordering. All three independent S3 object bodies must equal the literal rustack-sdk:<input>:<double> and DynamoDB numeric results42/46/50. Remove their objects before bucket deletion.

The negative resource configures the same valid server name but closed internal port9 as its gateway. Require production endpoint qualification to reject before ordinary readiness, the expected static startup diagnostic and nonzero server exit. An arbitrary early image/socket/config failure cannot substitute. After that verified exit, capture and validate the same actual owner.v1/daemon.v1 archives from the exact inspected persistent volume before any owner-scoped inventory or server/volume removal. Positive capture follows readiness; this negative capture follows its expected exit. Missing, malformed or unbound records fail and retain the volume/server/evidence, even if a broad inventory is empty. Verify all probe/runtime resources retire, then perform the same exact server/volume/source postconditions. Missing infrastructure fails; neither resource may skip.

Pure controls exercise invalid socket/GID input before dispatch; immutable server/mount/label/port-policy mismatches; matching record archives; wrong archive path/type/owner/mode/size/checksum/UUID/engine, duplicate/extra entries, bounded optional timing metadata and trailing concatenated data. Client-oracle controls reject wrong literal computations/counters, changed warm IDs and a host-published origin substituted for the internal DNS origin. Reuse existing real Runner reap/PID controls instead of duplicating them.

## 7. Build order and exit gates

1. Polish/review this design, source references and task5.3e ledger; no server-container resources run at the design checkpoint.
2. Add bounded server fixture image/Make provisioning and pure identity/policy/oracle controls. Reuse existing Runner/metadata/SDK patterns and tar workspace dependency; no new runtime dependency.
3. Implement the retained server-container owner, actual startup/identity binding, both ordinary consumers and always-joined exact cleanup. Prove all failures remain failures.
4. Freeze two exact ignored ordinary names before first execution: test_should_execute_image_sdk_and_nested_asl_from_a_server_container; test_should_refuse_unreachable_server_container_gateway_and_join_startup_cleanup. Preserve all existing names:41 ordinary/74 total/eight suites. Add discoverable test-workflows-server-fixture and test-workflows-server-docker targets and include provisioning/controls in the complete Docker gate.
5. Run both focused resources and all74 full resources on both retained engines under the final source cohort. Preserve fourteen actual source IDs per engine: D97's thirteen plus the server image; capture actual build exports and unchanged prebuild references. Reconcile the full Make footer with actual SDK/IaC status while this consuming gate changes; do not rewrite D97 evidence.
6. Run complete standard/strict/Native/security/text gates and independent source/evidence review. Prove zero all-state containers/volumes/private/test/pull references, all fourteen source IDs preserved and removal of only the two exact task-owned empty bridges.
7. Reconcile English guide/index/roadmap/capabilities and commit task5.3e. It closes server-in-container functional acceptance on the recorded lanes only. Native-x86, reference image/pure performance, Desktop/rootless and complete5.4 stay required.

## 8. Sources

- Docker [container run options](https://docs.docker.com/reference/cli/docker/container/run/) for explicit env files, supplemental groups, resource/security/network policy and publishing.
- Existing [LocalStack container/CI study](../docs/research/localstack-container-ci-research.md) for the separate server-container topology; it is prior art, not Rustack execution evidence.
- Pinned Moby [archive filesystem ownership](../vendors/moby/daemon/archive_unix.go) and [archive header construction](../vendors/moby/pkg/archive/archive.go); persistent volume records differ from D97's live tmpfs observer.
- [tar0.4.46 raw entry API](https://docs.rs/tar/0.4.46/tar/struct.Entries.html#method.raw), verified current on2026-10-31; it already exists in workspace dependencies. Bound input before parsing and never extract.
