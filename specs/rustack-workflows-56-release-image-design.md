# Workflows 56: Release Image and Action Qualification

Status: Task5.4a functionally qualified; executed evidence below · Updated: 2026-10-31 · Work item: D99 /5.4a

## 1. Outcome, dependencies and remaining release work

D99 consumes committed [D98](./rustack-workflows-55-server-container-design.md) (658ad61). Build the actual root Dockerfile for Linux arm64 and amd64; qualify its unchanged final image through the same real signed handler SDK/nested-ASL and startup-rejection consumers. Close release findings W371/W372/W374/W375 from [the canonical backlog](./rustack-workflows-93-review.md#release-packaging-backlog-for-task54). Preserve D98 artifacts and the source Cargo feature default. Acceptance comes from the actual root images and their required artifact/runtime consumers; see the executed evidence below.

The Docker distribution will compile Step Functions by default alongside all currently default-enabled services. The ordinary source build keeps its existing explicit stepfunctions feature; documentation must distinguish these two build profiles. Execution still requires the existing explicit Lambda executor configuration and real startup probes. Compiling a service never implies a ready Docker backend.

This is task5.4a, not the complete M4 exit. W373's same-commit/exact-image publication gate depends on the final platform/performance manifest and remains required in5.4b. D100 waives further Linux x86_64 validation; required arm64/reference/Desktop/rootless and remaining performance/load rows stay required. No image upload, remote workflow execution, git push, tag, global prune, context change or VM restart is authorized by this local qualification task.

## 2. Source build and final image contract

The root Dockerfile must copy patches/ before Cargo resolution, retaining the source-identical squib-loader security overlay. Align its official Rust builder version with rust-toolchain.toml (1.99.1 at this checkpoint). Keep --locked, Rust2024 and musl targets. Build the selected architecture's normal default CLI plus --features stepfunctions; do not silently add Step Functions to apps/rustack/Cargo.toml's source defaults or remove the security patch to make the build pass.

Retain scratch, CA roots, /rustack entrypoint, UID/GID65532, /data working directory, port4566 and the actual --health-check image command. Prepare /data mode0700/owner65532 and /tmp mode1777. Set ownership in the builder and copy the complete /runtime/ contents into final / as nested entries: copying each directory to a newly created top-level destination can reset that destination to0755. Verify the actual exported layer metadata, not only builder RUN permissions. Prepare the fixed /run/rustack/docker.sock placeholder in the builder and copy its parent tree into scratch before read-only execution. The only daemon socket remains an operator-supplied runtime mount, never an image layer. Do not bake test credentials, qualification YAML, a Docker client/daemon, user code or host executables into the release image. Exclude local env files, dependency/Python/Pulumi caches and Finder metadata from context without dropping required package sources/security patches. Limit source build jobs to default2, explicitly selectable1..8, for the retained VM memory budgets; record this build argument.

Record the selected builder manifest/compiler/target, actual image ID/platform and source revision. CI version updates in touched workflows must use current official releases with reviewed immutable source references; no remote CI result or publication is inferred from local YAML validation. Existing dedicated GNU fixture and relay/function sources remain required regression inputs.

Add discoverable Make variables WORKFLOWS_RELEASE_IMAGE (default rustack/workflows-release:integration-<arch>) and targets lambda-workflows-release-image, test-workflows-release-fixture and test-workflows-release-docker. Build with the already explicit selected Unix daemon/builder and --platform linux/<arch>, load locally and disable build provenance wrapping for the functional fixture cohort. The root Dockerfile remains the production recipe; do not replace its build stage with a host-prebuilt binary or test-only alternate Dockerfile. Builds precede runtime timing.

## 3. Reuse the retained server owner with an explicit image profile

Use a closed Fixture/Release profile selected by the Rust test, not arbitrary caller paths or shell fragments. Fixture selects the existing source variable/configuration; Release selects RUSTACK_RELEASE_IMAGE and the root-image contract. Keep all [D98](./rustack-workflows-55-server-container-design.md#5-ownership-failure-and-shutdown) budgets, directory/engine/nonce ownership, actual source pinning, CLI/Python PID joining, private env-file input, socket/GID/mount policy, independent identities, error retention and exact cleanup. No new Runner or process supervisor is needed.

```text
┌─ Make build / immutable-ID artifact gate ──────────────────────────────┐
│ same engine → bounded save → graph/hash/ELF/exact filesystem validation │
│ inherited kernel file cap; failure retains archive before runtime work │
└───────────────────────┬───────────────────────────────────────────────┘
                        ▼
┌─ retained host owner ───────────────────────────────────────────────────┐
│ explicit Fixture / Release profile                                     │
│ intent → owned volume → exact created server ID                         │
│ Release only: public YAML → bounded Docker cp → read-back verification   │
│ private env file → start → real readiness / expected rejection           │
└───────────────────────┬─────────────────────────────────────────────────┘
                        ▼
┌─ actual root-Dockerfile scratch image ───────────────────────────────────┐
│ /rustack + CA roots; UID65532; built-in --health-check retained           │
│ socket bind only + exact /data volume; read-only root; D98 limits         │
│ /data/qualification.yaml (public) → /data/state (private owner records)   │
│ internal DNS gateway ↔ normal relay/function pair                        │
└───────────────────────┬─────────────────────────────────────────────────┘
                        ▼
 SDK42/46 → parent .sync:2 → child Lambda50 → leave exact warm pair
                        ▼
 server TERM/wait → verify runtime retirement → remove exact server/volume
```

### 3.1 Public configuration inside the existing volume

The release image does not contain the fixture YAML. After exact server creation/inspection and before start, create a separate exclusive public YAML file under the private host root (mode0644, at most4096 bytes, credentials absent). Its contents are the existing fixed D98 budget profile. Use direct argv docker cp --archive <that-file> <exact-id>:/data/qualification.yaml; no stdin pipe, tar extraction, extra bind mount or change to Runner. Mark the mutation uncertain before dispatch and clear only after successful joined reply plus independent read-back verification. Never retry an uncertain copy.

Read back only that fixed basename through the existing bounded32KiB raw archive profile. Require one regular file, exact original bytes, UID/GID65532, mode0644, expected target volume affiliation, bounded timing PAX metadata and no extra/trailing entries. The original default-copy/root-ownership assumption failed on the actual engine: the default preserved the host501:20 archive owner. The explicit archive flag selects Moby copyUIDGID=true, whose tarCopyOptions derives ChownOpts from the independently validated numeric Config.User65532:65532. Require that exact destination ownership and0644 readability; do not accept arbitrary owner alternatives. Add a control requiring the actual copy argv flag, and reject root/host/wrong UID or GID in archive controls. Do not chmod the private env file to make it readable or put credentials into this public configuration. Set only the Release profile's RUSTACK_CONFIG=/data/qualification.yaml; DATA_DIR remains /data/state. Capture this intent and copy receipt privately before starting the server. Copy/read-back/schema failure fails qualification and retains uncertain authority/evidence.

### 3.2 Image health and readiness

Independently project/validate the selected source image's Config.User, WorkingDir, Entrypoint, Cmd and Healthcheck. Fixture retains its exact NONE health command. Release requires the exact exec-form [CMD,/rustack,--health-check], interval2s, timeout4s, start period1s and retries3, with no shell command or alternate executable. Keep the release healthcheck enabled in docker create; do not override it to mimic the fixture.

Release positive readiness requires both normal production capabilities and actual Docker health=healthy before the original180s readiness cutoff. An earlier unhealthy probe during real initialization is not successful readiness and does not reset that cutoff. The closed-port negative path still requires the precise production endpoint diagnostic/nonzero exit and persisted identity proof; it must never advertise ready. Preserve normal cleanup's non-OOM/non-dead/expected-exit checks and join health/server termination through the daemon's actual stopped state.

The signed SDK program, literal computations, independent S3/DynamoDB verification, nested parent/child history, warm IDs, no-socket/data leakage checks and error redaction remain shared with D98. Profile selection must not weaken their assertions.

### 3.3 Required actual image artifact gate (W391)

After every lambda-workflows-release-image build, Make invokes test-workflows-release-image on that selected local engine/reference; the latter can also inspect an already built image. Use a Python-standard-library fixture verifier under tests/workflows, reusing the tested Action Runner for every bounded direct CLI child. This adds no runtime container, mutable image alias, manifest resource or second process supervisor. Keep Docker76/Native26/Action1 unchanged. The release-fixture target also runs positive/adversarial archive controls.

Validate explicit arm64/amd64, a bounded local Unix endpoint, a non-option image reference and the selected WORKFLOWS_DOCKER_BIN (literal docker or bounded absolute executable path) before children. Start the original deadline before local filesystem observations. Capture the canonical daemon UUID; fence each operation on that endpoint, resolve the tag once to its immutable full image ID and save that ID with docker image save --output into a fresh0700 host root. Bound image metadata32KiB/strings4096/depth8, declared image size64MiB, saved archive128MiB (enforced before CLI dispatch through an inherited POSIX RLIMIT_FSIZE hard/soft cap, retaining any stricter inherited cap), at most128 outer members/eight layers, total expanded layer bytes256MiB and binary128MiB. CLI caps are info5s, inspect10s, save120s under one180s original cutoff; Runner retains its bounded pipes/TERM/KILL/physical-join behavior. An original POSIX alarm bounds local observations too; tighten the remaining post-save parse/report phase to10s without extending that original cutoff. Parent interruption follows the same retirement contract. Failure retains private evidence and is never accepted; no daemon runtime resource requires cleanup.

Before normalized tarfile iteration, enumerate every physical512-byte TAR header using bounded offsets and checksum-checked header decoding. Count metadata headers too; reject GNU longname/longlink/sparse/global-PAX and chained local metadata. Allow only one≤1024-byte local timing PAX immediately before its member, with≤3 unique mtime/atime/ctime keys and≤64-byte decimal timestamp values. Validate sizes, padding, canonical names and final zero terminators before normalized parsing; normalized member identities must match the raw table. Decode one bounded manifest.json and config, permit validated zero-size directories and bounded regular files only; reject duplicate/unsafe/oversized members, links/devices/FIFOs/other types, archive truncation and nonzero trailing material; never extract. Support bounded gzip or uncompressed saved layers, verify all uncompressed diff_ids and content-addressed blobs, and bind the saved graph to the captured config/OCI-manifest image ID. Preserve only the exact scratch filesystem: /rustack, CA root ancestors/bundle, /data, /tmp, /run and fixed socket ancestors/placeholder. Require data65532:65532/0700, tmp65532:65532/1777, runroot0:0/0555, nested run/rustack0:0/0555 and empty socket0:0/0444. The actual ELF must be64-bit/little-endian, match the selected Linux machine and contain no PT_INTERP. Require the same numeric user, working directory, entrypoint, absent Cmd/Volumes and exact image health contract as the runtime profile. Accept only Docker's fixed Linux PATH plus the three declared public image environment values; an implicit PATH is expected, not a leaked executable. Record hashes/identity/metadata under the private root. Final qualification independently binds these accepted artifact image IDs to the full gate's named build exports.

Controls require a valid graph for each machine and gzip/plain layers, then reject wrong ownership/mode/tree, architecture/interpreter, source/config/layer digest substitution, changed health/environment, links/duplicate/truncated/trailing archives and bounded decompression failures, including truncated gzip normalized to a fixed error. A real child inheriting a small test file limit must prove writes cannot exceed it; post-save fstat alone is insufficient. This audits the artifact actually consumed by Docker; builder RUN metadata alone cannot qualify it.

## 4. Composite Action input and readiness boundary

Keep the existing composite Action interface/defaults and caller-owned healthy container lifecycle. Docker image compilation adds Step Functions; add a stepfunctions-skip-auth boolean matching the existing service-specific convention and set STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION explicitly. An absent auth pair omits both server credential variables and their provider; enabled service skip-auth flags still govern. Reject an absent pair with an enabled strict flag before effects. The Action does not implicitly mount a daemon socket or opt a caller into code execution. Its default health/smoke path qualifies pure Step Functions; the separate release-container consumers qualify Docker Lambda.

All inputs cross into shell only through named step env entries. No ${{ inputs.* }} expression may appear in any run body. Quoted shell expansion is required; never use eval, shell-constructed command strings, unquoted arithmetic input, arbitrary environment names or a caller-selected registry/command. Validate every input in one first step before image lookup/pull, Docker allocation, outputs or exported environment mutation.

| Input | Boundary |
|---|---|
| image-tag | Docker tag grammar [A-Za-z0-9_][A-Za-z0-9_.-]{0,127}; fixed ghcr.io/tyrchen/rustack repository |
| port | canonical ASCII decimal1..65535; no leading zeros/sign/whitespace |
| wait-timeout | canonical decimal1..300 seconds; readiness budget only |
| default-region | exactly lowercase [a-z0-9][a-z0-9-]{0,63}, matching production settings; uppercase rejects before effects |
| log-level | exactly error/warn/info/debug/trace |
| all skip-auth flags; persistence | exactly true/false |
| services | empty or comma-separated supported service names, no whitespace/duplicates/empty members; max512 bytes/32 members; include stepfunctions |
| access-key / secret-key | both absent or both present; printable ASCII, each≤512 bytes, no control/CR/LF/NUL; values never rendered as shell source |

Use one Python3-standard-library Action runtime module under tools/github-action/ (outside the apps/* Cargo workspace glob). The composite run body only invokes that fixed module through a quoted trusted action-path env value; all input expressions occur in the step env map. It validates ASCII bytes/domain invariants before any child, image lookup or runner output mutation. Set LC_ALL=C for CLI diagnostics; never render a candidate value, exception traceback, raw Docker/server log or command trace. Nonempty credentials are registered through GitHub's percent-escaped masking command only; they do not enter ordinary diagnostics. This single entrypoint replaces the old multi-step Bash command owner; it is not a second Runtime API supervisor or an external dependency. Do not introduce shell scripts or duplicate the Rust server Runner.

Write Docker environment contents through a mode0600 file below an exclusive mode0700 RUNNER_TEMP child, with constant keys and bounded bytes (≤8KiB). Pass only its path with --env-file. Credentials never enter Docker argv, JSON intent or GITHUB_OUTPUT. Remove the input file after the physically joined run attempt, including failure; retain only nonsecret intent/identity evidence on uncertain effects. Export AWS/local endpoint/region/credential values through runner-owned GITHUB_ENV after successful readiness, with no multi-line injection possible. Open runner output paths with no-follow/nonblocking flags before checking their regular-file type so a FIFO cannot block handoff. Healthy service lifetime is explicitly handed to the caller through the validated full container-id output. Before that handoff the Action owns its created server and must attempt exact graceful cleanup on failure. Never force-remove, inspect/adopt a foreign existing name, or turn an ambiguous command into success.

### 4.1 One bounded direct subprocess owner

Capture one420s original launch cutoff after input validation. It covers local intent/env-file observations, daemon selection, image lookup/pull/resolution and Docker run; individual operations also have the following caps. Before dispatch and after every joined result, check the enclosing cutoff. No retry resets it. Readiness starts its separate validated wait-timeout cutoff immediately after the successful joined full-CID run reply; identity/publication checks consume that same readiness budget. Cleanup gets a separate90s cutoff. These budgets are compile-time constants, not new public tuning inputs.

| Operation | Maximum observation |
|---|---|
| Resolve local Unix endpoint; same-daemon identity fence |5s each, enclosing cutoff still applies |
| Image presence listing and projected image/container inspection |10s each |
| Missing-image pull |300s, docker pull --quiet; lookup errors never mean absence |
| docker run --detach |60s |
| Health HTTP | min(3s, remaining readiness); connect≤min(2s, remaining) |
| Cleanup TERM / final inspect / exact stopped-ID removal |10s each, cleanup cutoff |
| Cleanup daemon wait |≤70s and remaining cleanup |

Every child uses direct argv, null stdin, an isolated process group and independently drained bounded stdout/stderr (32KiB each; the health-body call alone allows64KiB stdout). Use one selector-driven helper in the Action runtime for all Docker/curl children. On deadline, output overflow, IO error or parent interruption: terminate that exact child group, allow≤1s grace, kill if necessary and physically wait/reap/drain/close within a further≤2s. Forced observation is always failure. A timed-out mutating Docker run may already have created a container: retain its pre-effect nonsecret intent and do not retry or delete by name. Actual controlled hung-child/descendant/flood cases must prove both pipes and processes are settled. No unbounded subprocess.run(...capture_output...) is permitted in the Action runtime. On Darwin an unreaped exited direct child can make group signal0 return EPERM. Reap only the owned direct child and independently retry group observation; do not reinterpret a persistent permission denial as proof of absence.

Resolve Docker's effective configured endpoint read-only after input validation, honoring explicit DOCKER_CONTEXT over DOCKER_HOST and require a local Unix endpoint, not TCP/SSH/npipe. Pin the resolved endpoint explicitly with --host on every later command; do not switch contexts. The Action requires a trusted local Docker engine with its loopback-published ports visible to the runner; it does not support arbitrary forwarded remote daemons. Capture the canonical nonnil daemon UUID and recheck it before/after each requested operation on the same selected endpoint. Missing/ambiguous/changed metadata fails rather than falling back.

Use an exact-reference bounded image listing to distinguish absence from CLI failure; only a successful empty listing permits one quiet pull. Then project the selected Linux image's full content ID/platform, and invoke the captured immutable ID rather than its mutable tag. Before run, write nonsecret intent with engine/source/expected name/port and a fresh UUID nonce. Add the exact nonce label at creation. The successfully returned64-lowercase-hex ID is recorded before further observations. A failed/ambiguous creation without that known identity never authorizes name-based inspection/removal.

### 4.2 Readiness affiliation and failure cleanup

Publish only127.0.0.1:<port>:4566 and preserve the http://localhost:<port> output contract. Every readiness iteration inspects only the returned full ID under the pinned daemon, requiring the exact image ID, nonce/name, running=true, dead=false, OOM=false, no restart policy and the exact unique4566/tcp→127.0.0.1:<port> publication. Require actual Docker health=healthy. Record StartedAt and ensure it is unchanged around the successful observation; never accept an exited/replaced/restarted producer or missing/foreign publication.

Between those exact pre/post inspections, request the fixed http://127.0.0.1:<port>/_health/ready URL through bounded direct curl argv, with --disable first to ignore ambient curl configuration and a separate exact HTTP200 status projection. Require200 and structured ready=true with a nonempty≤32-entry services map whose bounded ASCII names have only running/disabled states, matching the installed CLI's readiness semantics. Bound JSON to64KiB, depth8,512 total members,256-byte keys/strings; reject duplicate keys and malformed/late responses before acceptance. HTTP200 alone, ready=false, an unrelated body, a dead owned CID or a different published port cannot qualify. Recheck actual identity/health/publication after the HTTP result and then the original monotonic cutoff. Sleep at most0.5s/remaining budget. The outer runner step timeout is separate and cannot substitute for these finite child/readiness contracts.

On pre-handoff failure with a known verified CID, use only that same engine/full ID/source/nonce to request TERM, wait for actual exit and verify stopped/non-OOM/non-dead state before exact ID removal. Unexpected exit/forced command, changed identity, failed wait or uncertain mutation remains failure with retained intent/evidence; do not erase uncertainty because a later listing is empty. No volume or unrelated-container cleanup is performed. If output/env-file handoff fails, the server is still owned and this cleanup runs. On successful handoff, remove the private input/evidence root and leave the healthy exact container to the documented caller lifecycle.

## 5. Meaningful verification and required resources

Reuse the existing workspace YAML parser only as a test dependency edge (currently0.11.1; current upstream0.13.0 is observed, not silently substituted into production configuration). No new runtime parser/version change is needed to read trusted repository Action YAML. Bound the source to128KiB, one document,≤64 steps,≤128 env fields/step and≤16KiB per run body. Extract and execute the actual Action run block and its referenced real Python runtime in an isolated temporary root, with Docker/curl replaced by fixed controlled executables. A normal Rust contract test validates the actual YAML. A separate explicitly ignored Action process suite exports bounded run/env/default metadata under its private root, starts the real Python control suite through the existing independent Rust Runner, verifies its actual PID barrier and joins bounded capture under one180s cutoff. Failed/forced runs retain private reports and never qualify. The Python controls execute the actual body with an isolated environment/private cwd and the same bounded child helper; the independent Rust owner remains above that helper. The existing Rust Runner stays unchanged. Freeze the one Action suite in action-required-suites.json and verify its exact compiled ignored listing before execution, with positive/missing/renamed/extra/duplicate and cross-profile drift controls; this distinct action-process-controls profile does not add cases to Native26 or Docker76. Assert their argv/environment/output effects, not a rewritten copy of the Action logic.

Controls must prove malicious tag/port/timeout/region/log/services/boolean inputs dispatch no Docker/curl command and create no injected marker/output/env record; printable credential metacharacters stay literal, never execute, never enter Docker argv or diagnostics; valid defaults/custom values export the correct endpoint and credentials; partial/oversized/multi-line credentials reject; a foreign existing name is not deleted; true bound readiness succeeds; missing/failed/late/ready=false/unrelated200 health, exited owned CID and wrong publication fail; known failure cleans only its exact stopped CID while unknown creation retains authority; hung/flooding child groups are killed and joined; the advertised timeout uses real elapsed seconds; all children/pipes are joined. Add a pure STANDARD Step Functions SDK/CLI computation and exact history assertion to the existing default Action CI consumer (MathAdd input21→42); a service-count check cannot prove compilation/execution. YAML/profile/context controls also prove patches present, toolchain alignment, Step Functions compilation, unchanged source defaults and correct final socket/data/health policy.

Freeze the two new ordinary ignored resource names before execution:

- test_should_execute_image_sdk_and_nested_asl_from_the_release_image
- test_should_refuse_unreachable_release_image_gateway_and_join_startup_cleanup

The manifest becomes43 ordinary/76 total/eight suites; keep Native's26-resource inventory unchanged. Extend normal fixture provisioning and a focused target without bypassing any existing exact inventory. Require19 existing Python controls/12 existing Rust controls plus new focused policy/archive/Action controls; do not replace existing consumers with release-only variants. Both old GNU and new root-image resources run in the full gate. Existing journal corruption prerequisites must guarantee a changed field even when an original UUID byte is0xff; a random no-op must not masquerade as an invalid-state decoder failure. Use a fixed0xff owner boundary fixture plus a guaranteed different owner byte, preserving the remaining malformed canonical fields and production codec.

## 6. Dependency-ordered implementation and exit (executed D99)

1. Polish this design, phase ledger, key decision and verification contract; independently review before source changes. D98 is committed/frozen and must not be rerun or overwritten.
2. Correct root build context/pin/profile/filesystem; implement Action env/validation/privacy/deadline changes and meaningful actual-body controls. Run targeted controls/strict lint first.
3. Add the explicit release owner profile, public-config copy/read-back and exact image-health validation with adversarial controls. Preserve existing profile behavior and Runner.
4. Build both actual root images; capture provenance/ELF/platform and actual sources. Retain every failed build/run as its original cohort; never repair failure evidence to green.
5. Freeze a complete new source cohort,43/76/eight manifest and fifteen source identities per engine (the prior fourteen plus one actual root release image). Run both focused resources and every full76 resource on both retained engines.
6. Run all standard Rust default/feature build/test/fmt/Clippy/rustdoc, strict boundaries, complete Native, fresh audit/deny and text checks on that same source. Independently review source/evidence; verify zero remnants and preserve all fifteen sources, removing only exact task-owned empty bridges.
7. Reconcile English guide/capabilities/roadmap/current summaries and commit task5.4a. W371/W372/W374/W375 close only with actual acceptance. W373 and required arm64/reference/Desktop/rootless/remaining performance/load/full5.4 remain required under D100; no local image publication or push occurs.

## 7. Sources and prior art

- [D98 retained server owner](./rustack-workflows-55-server-container-design.md), [runtime ownership](./rustack-workflows-20-lambda-runtime-design.md), [operations](./rustack-workflows-60-operations-design.md) and [verification](./rustack-workflows-70-verification-plan.md) bind shared behavior.
- [Existing container/CI research](../docs/research/localstack-container-ci-research.md) supplies prior art, not Rustack release-image evidence.
- [GitHub secure use](https://docs.github.com/en/actions/reference/security/secure-use) specifies intermediate environment variables, quoting and secret masking. The input validator and private env-file contract above are Rustack's own stricter boundary.
- [Docker test before push](https://docs.docker.com/build/ci/github-actions/test-before-push/) explains local load→test→publish ordering; its simple example does not prove both architectures or Rustack's complete release requirements.
- [Current build action](https://github.com/docker/build-push-action) and [releases](https://github.com/docker/build-push-action/releases) observe v7.4.0; [checkout releases](https://github.com/actions/checkout/releases) observe v7.0.1. These are source references, not a claim that any CI run has occurred.
- Current official release metadata and git tag resolution pin the touched CI consumers to [checkout7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1), [build-push7.4.0](https://github.com/docker/build-push-action/releases/tag/v7.4.0), [setup-buildx4.4.1](https://github.com/docker/setup-buildx-action/releases/tag/v4.4.1), [upload-artifact7.0.2](https://github.com/actions/upload-artifact/releases/tag/v7.0.2) and [download-artifact8.0.2](https://github.com/actions/download-artifact/releases/tag/v8.0.2). Use their complete resolved commit IDs with version comments. Only the existing test-action workflow and its local artifact loader are touched; this is no remote execution/publication evidence.
- [Current YAML API](https://docs.rs/yaml-rust2/latest/yaml_rust2/) observes0.13.0. The test reuses this repository's existing audited workspace version; a production YAML upgrade is outside this dependency edge.


## 8. Executed qualification and scope update

The final1250-source cohort passes both actual corrected root-image artifact audits and focused SDK/nested/startup-rejection consumers, both complete76-resource functional gates, standard/strict/Native/security/text and independent source/evidence review. Full gates pass558 aggregate Rust cases/33 summaries per lane; focused gates pass484/27. Artifact13, actual Action26 and shared Rust-policy16 controls remain separate from Docker76/Native26/Action1 inventories. Exact postconditions prove zero runtime remnants, fifteen actual source IDs per engine and removal of only two exact task-owned empty bridges. [Final D99 evidence](./rustack-workflows-93-review.md#d99-final-qualification-and-exact-postconditions) records original failed cohorts, exact hashes and cleanup authority.

[D100](./rustack-workflows-99-decisions.md#d100--linux-x86_64-validation-is-waived) records the subsequent user instruction to perform no further Linux x86_64 validation. Both-architecture commands/contracts above describe the completed D99 cohort, not future mandatory x86 work. W373 and required arm64/reference/Desktop/rootless/performance/load/full-M4 work remain open. No remote CI result or publication is claimed.
