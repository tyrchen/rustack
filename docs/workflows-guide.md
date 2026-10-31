# Step Functions and Lambda workflows

Rustack runs STANDARD workflows through its own ASL engine. The opt-in
`stepfunctions` source feature supports a bounded JSONPath profile, all eight
state types, retries/timeouts, Inline Map/Parallel, nested workflows and Lambda
callbacks. Real Lambda execution is qualified through the explicit trusted-host
Native backend and the ordinary Docker image startup path. Docker requires the
configured ownership/recovery and real kernel/network/optional gateway probes
below; it uses the actual shared provider for SDK and ASL calls. The configured
provided.al2023 profile executes original ZIP code and restores bounded ZIP/Image
archives through the same qualified owner. Official Python, pinned Node24 and an
independent Rust RIC have functional evidence on the retained lanes. Explicit
local SDK access to S3/DynamoDB, real image callbacks and managed Pulumi IaC
consumers and the separate Linux server-container topology are qualified below.
The actual root release image and composite Action are qualified through D99.
W373 promotion, required arm64/reference/Desktop/rootless, remaining performance/load
and complete release qualification remain unfinished. The user has waived
further Linux x86_64 validation; recorded amd64/QEMU results are historical
functional evidence, with no native-x86 performance claim.

## Run the pure example

Run the commands from the repository root, using Cargo and the pinned Rust toolchain. The qualification commands require a Unix host
(Linux or macOS); performance evidence includes macOS/aarch64 and a native arm64 Linux VM. The feature is absent from the default source build;
adding `SERVICES=stepfunctions` to that unchanged binary cannot enable it.

```bash
# Compile the Step Functions service without Lambda or other service adapters.
cargo build --locked -p rustack-cli --bin rustack \
  --no-default-features --features stepfunctions

# Run the checked-in strict-auth configuration; only dummy local credentials apply.
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  RUSTACK_CONFIG=examples/workflows/rustack.yaml \
  cargo run --locked -p rustack-cli --bin rustack \
    --no-default-features --features stepfunctions -- --snapshot workflow-demo
```

The server listens on `127.0.0.1:4566`. The
[example YAML](../examples/workflows/rustack.yaml) enables only Step Functions,
requires local SigV4 credentials and sets enforced workflow budgets. Process
environment overrides YAML; conflicting region/account/credential aliases fail
startup. `RUSTACK_CONFIG` selects the file; there is no `--config` flag.

In another terminal, use an installed AWS CLI v2. The explicit endpoint applies to
each request, as described in the [AWS CLI endpoint guide](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-endpoints.html).

```bash
export AWS_ACCESS_KEY_ID=local-key AWS_SECRET_ACCESS_KEY=local-secret
export AWS_DEFAULT_REGION=us-east-1 AWS_REGION=us-east-1 AWS_PAGER=""
unset AWS_SESSION_TOKEN
export WORKFLOW_ENDPOINT=http://127.0.0.1:4566

aws --endpoint-url "$WORKFLOW_ENDPOINT" stepfunctions validate-state-machine-definition \
  --definition file://examples/workflows/pure.asl.json --type STANDARD

aws --endpoint-url "$WORKFLOW_ENDPOINT" stepfunctions create-state-machine \
  --name Documented --type STANDARD \
  --role-arn arn:aws:iam::000000000000:role/Workflows \
  --definition file://examples/workflows/pure.asl.json

aws --endpoint-url "$WORKFLOW_ENDPOINT" stepfunctions start-execution \
  --state-machine-arn arn:aws:states:us-east-1:000000000000:stateMachine:Documented \
  --name Positive --input '{"n":6}'

aws --endpoint-url "$WORKFLOW_ENDPOINT" stepfunctions describe-execution \
  --execution-arn arn:aws:states:us-east-1:000000000000:execution:Documented:Positive

aws --endpoint-url "$WORKFLOW_ENDPOINT" stepfunctions get-execution-history \
  --execution-arn arn:aws:states:us-east-1:000000000000:execution:Documented:Positive
```

Start returns acceptance; describe again if it is still RUNNING. The checked-in
[ASL](../examples/workflows/pure.asl.json) doubles `n`, selects the positive branch,
waits zero seconds and succeeds with the JSON output `{"doubled":12}`. Starting a
different execution name with `{"n":0}` reaches Fail with `Example.NotPositive`.
Output is a JSON-bearing string in the API response. The role ARN is validated
and captured locally; this is not IAM policy enforcement. CLI argument shapes
follow the official [create](https://docs.aws.amazon.com/cli/latest/reference/stepfunctions/create-state-machine.html)
and [start](https://docs.aws.amazon.com/cli/latest/reference/stepfunctions/start-execution.html)
commands; Rustack implements the narrower profile below.

The automated equivalent owns its server, uses the same YAML/ASL files and an
ordinary signed Rust SDK, tests both branches and joins server cleanup:

```bash
make smoke-workflows
```

This smoke needs Cargo and Python 3 for inventory validation. It does not need a
running server, Native bootstrap, `uv`, boto3 or Docker. Other SDK acceptance
targets deliberately provision their own additional fixtures.

## Run Docker image Lambda tasks

Build with `--features lambda,stepfunctions` (the default source feature set also
contains Lambda). Explicit `LAMBDA_EXECUTOR=docker` selects the real image runtime;
execution otherwise stays disabled. Supply an operator-selected relay artifact,
existing user bridge or explicit `none`, local Unix daemon socket and a private
absolute `DATA_DIR`. The existing data-directory default is `/var/lib/localstack`;
choose a writable private directory for local development. Example YAML:

```yaml
environment:
  SERVICES: "lambda,stepfunctions"
  GATEWAY_LISTEN: "127.0.0.1:4566"
  DATA_DIR: "/absolute/private/rustack-data"
  LAMBDA_EXECUTOR: "docker"
  LAMBDA_DOCKER_SOCKET: "unix:///var/run/docker.sock"
  LAMBDA_DOCKER_RELAY_IMAGE: "rustack/lambda-runtime:integration-arm64"
  LAMBDA_DOCKER_NETWORK: "none"
  LAMBDA_SKIP_SIGNATURE_VALIDATION: false
  STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION: false
  ACCESS_KEY: "local-key"
  SECRET_KEY: "local-secret"
budgets:
  startupSeconds: 600
  shutdownSeconds: 30
```

The relay tag is an explicit locally built operator source, not a runtime catalog.
Build it for the selected daemon architecture with
`make lambda-runtime-image WORKFLOWS_ARCH=arm64 WORKFLOWS_DOCKER_SOCKET=<socket-path>`;
choose the installed Docker buildx command through `WORKFLOWS_IMAGE_BUILDER` if
needed. The actual image reference defaults to the architecture-suffixed integration
tag shown above. Image sources must already exist unless you explicitly configure
the exact registry allowlist and `ifMissing` pull policy in the
[operator specification](../specs/rustack-workflows-60-operations-design.md#11-docker-configuration).
No ambient registry credential helper is executed. A private credential file has
its own bounded quoted YAML schema, ownership/mode checks and operator-controlled
ACL precondition.

The daemon must be Linux and expose the required enforced resource/isolation
controls. Startup resolves its selected arm64/amd64 platform and pins the relay
by captured immutable identity. With cross-architecture false, the host and daemon
architectures must match. Explicit true permits the separately probed selected
target, including a remote emulated daemon; other function architectures are
rejected during preparation. Pass `Architectures=["arm64"]` or `["x86_64"]` when
creating an Image function to match the qualified platform. Use the same ordinary
Lambda and Step Functions clients/ASL integrations described in this guide.

`none` deliberately permits disconnected functions with `workers.docker.sdkGateway=false`.
For function-originated service access, select an existing user bridge and set
`LAMBDA_DOCKER_GATEWAY_ENDPOINT` to the independently chosen function-visible origin.
Choose a gateway listener reachable from that network; default host loopback is
not generally sufficient. Startup runs the trusted pinned function in that exact
network and requires a response from this listener instance. No endpoint is
inferred from Host or the advertised URL. HTTP is operator-trusted local networking;
HTTPS uses normal name/certificate verification with embedded public roots, without
production private-CA loading. This reachability receipt is separate from the actual handler SDK and callback
acceptance described below. Configure the explicit local SDK pair deliberately;
host AWS secrets are never inherited.

Before readiness, the actual listener serves bounded diagnostics only. The fixed
`/_rustack/runtime-probe` identity is public correlation, not authentication.
Ordinary ingress opens only after recorded previous-run cleanup, complete bounded
namespace inventories, live pins, actual kernel/optional endpoint receipts, provider
construction and snapshot validation. `workers.docker` reports ready, platform,
sdkGateway and zip=false. Backend/deployment owner faults affect readiness;
normal FunctionError does not. One startupSeconds deadline includes all startup
work, independently of shutdownSeconds.

New ownership identities carry the journal-generation magic. Legacy identities
are preserved and refused for production; an empty scan cannot upgrade them.
Retain old evidence and choose a fresh controlled DATA_DIR when appropriate; this
does not reclaim old resources. Never delete/reset owner, lock, daemon binding or
journals to bypass refusal. Shutdown retains exact cleanup ownership and removes
only recorded private resources, preserving operator source images/cache.

An invocation whose function memory plus the64MiB relay charge exceeds
`lambdaDockerMemoryBytes` returns429 before allocation. Temporary contention
uses the bounded runtime queue. For a qualified Docker backend, shutdown reserves
`lambdaDockerCleanupSeconds`, capped at half of `shutdownSeconds`, for final
cleanup inside the original shutdown deadline. Native/pure workflows keep their
existing grace period. An unfinished accepted-work drain still exits with an
error and does not publish a successful snapshot, even if container cleanup
succeeds.

ZIP deployment staging bounds compressed code to50MiB, expanded content to250MiB,
and explicit entries plus implicit parent directories to10,000 filesystem nodes.
It rejects traversal/absolute/drive/NUL names, links, special file declarations,
privileged modes and corrupt directory/file streams.

Docker runs Image functions by default. The explicitly configured provided-runtime
profile below adds ZIP execution and managed ZIP archives. Named Image snapshots
restore resources, published versions, aliases and terminal workflow history without replay. The archive stores
image references and execution metadata, not image layers. Every usable revision
requires the exact saved immutable content to remain in the local daemon; restore
never pulls or follows a changed source tag. The provider account and region must
match. Fresh platform/fingerprint/size proof precedes publication and ingress.
Invalid metadata, missing content or an expired startup deadline preserves both
current and previous archive generations.

Image-only Docker Lambda metadata is capped at16MiB,256 functions and1024 total
latest/published revisions, with bounded depth, members and scalar sizes. Saving
over-profile metadata fails before generation publication. The managed ZIP profile
retains the same metadata/count limits and adds separate original-code budgets below. See the
[Image archive contract](../specs/rustack-workflows-20-lambda-runtime-design.md#216-candidate-store-image-archive-restoration-d88)
for exact limits. Named Native/pure workflow snapshots remain available within
their documented profile. The L01–L09 Docker fault/admission matrix is functionally
qualified on the retained lanes, including the pinned Node24 consumer below.
Image SDK access to S3/DynamoDB and image callbacks also pass on those lanes.
Managed Pulumi IaC also passes on those lanes. Server-in-container, native
reference and final platform/release gates remain open.

The explicit `make test-lambda-startup-docker` gate builds its host executable in
an isolated Cargo target directory and captures a fixture-owned executable copy,
so concurrent plain/minimal builds cannot replace it. It requires the explicit
socket, architecture, fixture images and (for connected cases) an owned user bridge
and function-visible host. Missing prerequisites fail; it uses actual startup
receipts and ordinary signed SDKs, without injected readiness.

The custom Rust RIC acceptance fixture is built explicitly with
`make lambda-custom-ric-images` for the chosen architecture/socket. The
`test-lambda-custom-ric-docker` target runs the complete ordinary startup suite,
including signed Rust SDK computation and boto3 ImageConfig, mutable-tag/update,
publication/alias and active cross-invocation identity cases. Supply the same
explicit owned bridge and function-visible host as endpoint qualification. Its
bounded fixture-only collector holds both consumed requests until each rejects
foreign/stale completions; it also proves a published A call finishes as A after
latest updates successfully to B. Cold published-A/latest-B launches must also
return counter=1 after other resources displace their environments. Test source tags are uniquely owned and checked
before removal. The collector is a test barrier, not a required user endpoint or
handler SDK compatibility claim. Official Python, pinned Node24 and independent
Rust RIC and image callback evidence covers the recorded retained lanes;
the remaining platform matrix stays open. Collector failure controls run with `make test-lambda-custom-ric-fixture`.

The pinned official Node24 handler image has a separate explicit consumer:

```bash
make test-lambda-node-docker \
  WORKFLOWS_ARCH=arm64 \
  WORKFLOWS_DOCKER_SOCKET=/absolute/selected/docker.sock \
  WORKFLOWS_NETWORK=existing-user-bridge \
  WORKFLOWS_GATEWAY_HOST=192.168.5.2 \
  WORKFLOWS_DOCKER_BIN=/absolute/path/to/docker \
  WORKFLOWS_IMAGE_BUILDER=/absolute/path/to/docker-buildx
```

This builds the digest-pinned Node24 base with the checked-in built-in-only
handler and exercises signed Rust/boto3 calls and an optimized ASL Lambda task.
It checks actual Node version/architecture/UID, read-only code, writable temporary
storage, stdout/stderr tails, cold/warm counters, published versions/aliases after
CMD updates and named FunctionError followed by a fresh healthy replacement.
`make test-lambda-node-fixture` runs its observer, child-join, Runtime HTTP and session-admission
controls without allocating Docker resources. Node ZIP/runtime-name support is
outside this image profile. Both retained functional lanes execute Node24.22.1.
See the [official Node contract](../specs/rustack-workflows-20-lambda-runtime-design.md#417-official-node-image-consumption-d94)
for exact ownership and validation bounds. Required arm64/reference and Docker
Desktop/rootless remain separate requirements; image callback acceptance is documented below.

The default Node RIC can retain five normal keepalive sockets. The local Runtime
API allows eight connection tasks, one active invocation/poll and four charged
POST bodies of at most6MiB each. Admission remains charged while an accepted
payload is queued even if its HTTP observer expires. General header values remain
256 bytes within16KiB aggregate; only the exact FunctionError X-Ray cause field
allows up to8KiB. Rejected metadata does not settle the invocation. Custom images
with larger worker/socket pools must fit these bounds.

Image and provided-runtime functions can opt into local AWS SDK access. Configure
both `LAMBDA_DOCKER_SDK_ACCESS_KEY_ID` and
`LAMBDA_DOCKER_SDK_SECRET_ACCESS_KEY`, together with the existing qualified
`LAMBDA_DOCKER_GATEWAY_ENDPOINT` and a selected user bridge. The pair must match
the local services' credentials. Build with `lambda,stepfunctions,s3,dynamodb`
and explicitly enable those services when using S3/DynamoDB. Server credential
aliases must agree; differing `ACCESS_KEY` and `AWS_ACCESS_KEY_ID` fail validation.

Startup issues the SDK context only after its actual same-instance endpoint
proof. The function receives the explicit pair and canonical `AWS_ENDPOINT_URL`;
profile, session, role, metadata/container credential and per-service endpoint
selectors are cleared, including inherited image defaults. SDK clients can use
their default credential/endpoint providers. S3 clients should use path-style
addressing for the local gateway. Client code can still explicitly select other
credentials or endpoints. With the pair absent, existing user environment
behavior is preserved. Partial pairs fail startup; host credentials are never
used as a fallback for this context.

The injected pair belongs to the installed runtime and its function containers.
It does not enter relay/probe environments, GetFunctionConfiguration, captured
revisions or saved archives. A new installation can supply a different pair;
restored function metadata cannot choose it. `sdkGateway` continues to report
qualified reachability, rather than authorization for a particular service.
See the [local SDK contract](../specs/rustack-workflows-20-lambda-runtime-design.md#418-explicit-local-handler-sdk-environment-d95)
for limits, affiliation and environment precedence.

`make test-lambda-handler-sdk-docker` uses the same explicit socket, architecture,
owned bridge, function-visible gateway, Docker CLI and image-builder settings as
the full gate below. It builds a separate pinned Node24 SDK image and runs two
exact resources: real default-provider S3/DynamoDB cold/warm read/write with
independent host verification, and wrong explicit credentials with precise
authorization errors despite valid ambient server credentials. Its quiet-body
control checks cancellation after HTTP headers. The pinned image's actual
clients are 3.1105.0; no npm installation is needed.
`make test-lambda-handler-sdk-fixture` runs the configuration, environment,
affiliation, bounded archive-omission and observer controls without Docker
resources. Both retained functional lanes pass these SDK consumers. After the
fixture images have been provisioned, `make test-lambda-image-sdk-consumers-docker`
repeats the Node/SDK controls, complete compiled manifest and all three exact
Node/SDK resources using those recorded images. It does not rebuild images;
missing prerequisites fail. Image callback acceptance is documented next;
remaining reference/platform gates are separate requirements.

`make test-lambda-callback-docker` builds a separate digest-pinned Node24 callback
image and runs two exact resources covering all nine callback cases. Supply the
same selected/other Docker sockets, architecture, bridge, gateway and builder
settings as the full gate below. `make test-lambda-callback-fixture` checks the
safe gates and independent privacy/archive observers without allocating Docker
resources.

The callback Task invokes dispatcher A synchronously. For post-return completion,
A uses the actual default-provider Lambda SDK to accept worker B with Event/202;
B stays in its own active invocation while A returns. Safe DynamoDB gates then
release B to call SendTaskSuccess or SendTaskHeartbeat; gated A supplies the
early success/failure cases with SendTaskSuccess or SendTaskFailure. This
requires no work to continue after A's managed handler returns. Tests prove
before/after-return causality, unrelated Lambda progress with one workflow
dispatch slot, fresh tokens on retry, heartbeat versus absolute timeout and
Stop cancelling A while independent B remains owned until released and joined.

Both retained functional lanes pass real Lambda/Step Functions/DynamoDB
clients3.1105.0, exact SDK error classes and all nine catalog cases. Confirmed
saved archives retain the two actual functions and terminal workflow rows;
bounded history/log/archive scans reject leaked tokens or injected credentials.
See the [image callback contract](../specs/rustack-workflows-40-integrations-design.md#56-real-image-sdk-callbacks-and-independent-event-worker-d96).
This is native-arm64 VZ/emulated-amd64 QEMU functional acceptance; required arm64
reference, Docker Desktop/rootless and complete release gates remain open.

`make test-workflows-iac-docker` builds the pinned Pulumi tool image and runs two
real consumers through the ordinary service. Supply the same architecture,
selected/other sockets, bridge, gateway and builder settings as the full gate
below. `make test-workflows-iac-fixture` runs dependency, artifact, program,
privacy, replay and process-ownership controls without allocating Docker
resources. Provisioning stays in the project fixture/cache; no global Pulumi CLI
or user plugin cache is used.

The tool function runs CLI/core3.273.0 and AWS provider7.52.1 inside the managed
image namespace. A separate Node image supplies actual business computation.
The lifecycle runs create/refresh/update, three explicitly rejected unsupported
updates, valid recovery followed by strict no-change convergence, destroy and
stack removal. Independent clients require42/46 results with new execution and
request identities, and observe the actual API resource before and after each
transition. A scoped existing unpublished machine supports an empty
ListStateMachineVersions read; publication, qualified version ARNs and aliases
remain unavailable.

State audits distinguish encrypted provider inputs before destroy, actual
current state/API absence after destroy, and active-checkpoint absence after
stack removal. A pinned read-only observer reads the live tmpfs and must finish
successfully before host decoding. The Stop consumer observes actual CLI,
language and provider processes alive, retires only the tool function/relay,
and verifies that the exact business pair remains healthy. The private test
profile uses two environments, two workflow dispatches and a1,536MiB aggregate
memory ceiling. See the [IaC contract](../specs/rustack-workflows-50-iac-design.md).
At D97, both retained functional lanes passed the lifecycle and Stop resources, all72
full Docker resources and exact zero-remnant/source-preservation checks. See
[final D97 qualification](../specs/rustack-workflows-93-review.md#d97-final-qualification-and-exact-postconditions).
D98 qualifies the separate server-container topology below. Required arm64
reference and additional Docker platforms remain required.

`make test-workflows-server-docker` builds the actual Linux Rustack CLI in a
separate server fixture image and runs two exact resources. In addition to the
full gate's inputs, explicitly supply `WORKFLOWS_DOCKER_DAEMON_SOCKET` and
`WORKFLOWS_DOCKER_DAEMON_SOCKET_GID`: these describe the socket path and group
inside the selected Linux daemon host. They are distinct from the host client's
Unix socket. No macOS-to-VM path inference is performed. The image build uses
the pinned toolchain, existing GNU targets, cargo-zigbuild and Zig0.16.0; a
bounded compatibility adapter handles its obsolete linker optimization flag.

The server runs as UID/GID65532 with the supplemental socket group, a read-only
root, dropped capabilities and a dedicated named data volume. Only that trusted
server receives the socket. Its internal bridge DNS address supplies the handler
SDK endpoint; the host client uses a separately inspected loopback-published
port. Real signed SDK calls compute42/46, and a nested `.sync:2` child computes50
through the same warm function/relay pair. The negative case must reject a
closed internal gateway port during production startup. Both paths validate
the actual persisted owner/engine records and normal server/runtime shutdown
before removing their exact server and volume. They never force-clean a failed
production shutdown into a passing result.

Both focused resources and the complete74-resource gate pass on native-arm64
VZ and emulated-amd64 QEMU, with535 Rust cases per full lane. Exact postconditions
prove zero remnants, fourteen preserved source IDs per engine and removal of
only the two task-owned empty bridges. See [final D98 qualification](../specs/rustack-workflows-93-review.md#d98-final-qualification-and-exact-postconditions).
`make test-workflows-server-fixture` runs the import, history, identity,
archive, policy, privacy and diagnostic controls without Docker allocation.
See the [server-container contract](../specs/rustack-workflows-55-server-container-design.md).
This fixture topology does not qualify the default release Dockerfile or close
required arm64/reference/Desktop/rootless/final-release requirements.

## Build and qualify the release image

The root `Dockerfile` now builds the normal default CLI plus `stepfunctions`
with the pinned Rust toolchain and the repository's security patch. This Docker
distribution profile differs from the opt-in source feature above. Compiled
Step Functions supports pure ASL without a Lambda executor; real code execution
still requires explicit backend settings and successful startup probes.

```bash
make lambda-workflows-release-image \
  WORKFLOWS_ARCH=arm64 \
  WORKFLOWS_DOCKER_SOCKET=/absolute/selected/docker.sock \
  WORKFLOWS_IMAGE_BUILDER=/absolute/path/to/docker-buildx
```

The default local tag is `rustack/workflows-release:integration-arm64`; choose
`amd64` for the other architecture. `WORKFLOWS_RELEASE_BUILD_JOBS` defaults to2
and accepts1..8. The production recipe builds musl inside the selected Linux
engine; it keeps scratch, UID65532, CA roots and its real image healthcheck.
Builds/pulls are separate from runtime timing. Every build also runs the
`test-workflows-release-image` gate: it saves the captured immutable image ID
without starting a container, checks the actual static ELF and exact scratch
filesystem/health profile, and retains a private artifact receipt. The same
target can recheck an existing image. Kernel file limits, bounded decompression
and finite CLI observations guard this inspection.

`make test-workflows-release-docker` accepts the same explicit daemon-side
socket/group, network, gateway, other daemon and builder inputs as the complete
gate below. It builds the actual root image and runs two exact resources:
signed SDK42/46 plus nested `.sync:2` child50, and unreachable-gateway startup
rejection. It reuses the server owner and places only public budget YAML into
the exact data volume; private credentials remain in a separate env file.
Release acceptance requires actual Docker health, persisted identity proof and
normal server/runtime cleanup. Source/build completion alone is insufficient.

The composite GitHub Action exposes `stepfunctions-skip-auth` with the existing
service conventions. Its default container does not receive a daemon socket;
pure workflows work without enabling function code execution. All inputs enter
the runtime through environment values and are validated before effects.
Readiness uses elapsed time, exact container/image/daemon identity and structured
health. A healthy full container ID is handed to the caller for cleanup.
`make test-workflows-action-fixture` runs26 actual-body adversarial/process
controls under an independent Rust process owner and an exact one-resource
Action manifest. This Action profile is separate from Native26 and Docker76.
The default Action CI consumer also checks pure ASL21→42 and its exact history;
local YAML/control verification is not a remote CI result.

The [release-image contract](../specs/rustack-workflows-56-release-image-design.md)
records the required both-lane acceptance and remaining publication/platform
work. Both actual image profiles pass final D99 qualification: all76 resources
and558 aggregate Rust cases per full lane, with zero remnants and fifteen
source identities preserved per engine. See [final D99 evidence](../specs/rustack-workflows-93-review.md#d99-final-qualification-and-exact-postconditions).
[D100](../specs/rustack-workflows-99-decisions.md#d100--linux-x86_64-validation-is-waived) waives further Linux x86_64 validation.
Existing both-lane commands/results describe completed evidence; future required
platform work uses arm64 and the remaining Desktop/rootless/reference gates.

`make test-lambda-faults-docker` runs that complete startup suite plus real
initialization/error/exit/OOM/timeout, signed initializing/busy caller
disconnects, reserved quotas and128 accepted Event requests under two actual
environment pairs. Its held backlog must report unfinished draining while
still cleaning owned containers before natural expiry. The bounded observer
controls run with `make test-lambda-fault-fixture`; library memory/admission
regressions run with `make test-lambda-faults`. These gates require the same
explicit images, daemon, bridge and function-visible host. The full gate below also qualifies allocation/cleanup IO faults, daemon loss
and pull/deployment saturation on the retained functional lanes.

The complete current Docker functional resource gate is explicit:

```bash
make test-lambda-docker \
  WORKFLOWS_ARCH=arm64 \
  WORKFLOWS_DOCKER_SOCKET=/absolute/selected/docker.sock \
  WORKFLOWS_DOCKER_DAEMON_SOCKET=/var/run/docker.sock \
  WORKFLOWS_DOCKER_DAEMON_SOCKET_GID=991 \
  WORKFLOWS_OTHER_DOCKER_SOCKET=/absolute/other/docker.sock \
  WORKFLOWS_NETWORK=existing-user-bridge \
  WORKFLOWS_GATEWAY_HOST=192.168.5.2 \
  WORKFLOWS_DOCKER_BIN=/absolute/path/to/docker \
  WORKFLOWS_IMAGE_BUILDER=/absolute/path/to/docker-buildx
```

Choose values for your actual two distinct daemons, daemon-side socket/group and function-visible gateway;
the addresses above are placeholders. The gate builds fixtures, compares eight
compiled inventories against the [frozen 76-resource manifest](../tests/workflows/docker-required-suites.json), then runs every
resource sequentially. Missing/extra/renamed ignored tests fail preflight.
It includes the standalone relay suite and real cross-daemon binding; run each
selected architecture with the other daemon supplied. This qualifies the
`docker-image-functional` profile, while final platform/release requirements
remain open. `make test-lambda-docker-manifest` checks inventories without
running Docker resources.

Docker ZIP execution is an explicit provided-runtime installation. Set
`LAMBDA_DOCKER_ZIP_IMAGE` to a locally available immutable `repository@sha256:...`
OS image for `provided.al2023`, alongside the relay image. There is no implicit
base catalog or fallback runtime. Deploy a ZIP with a root executable Linux ELF
`bootstrap`, `Runtime=provided.al2023`, an ASCII handler of1..128 bytes using
letters, digits or `_./:-`, and one architecture matching that installed image.
The handler is passed as `_HANDLER`; Rustack always starts `/var/task/bootstrap`.
Original ZIP SHA256/size remain the public code identity. Validation failures do
not replace the latest revision, publications or aliases.

Use a fresh data directory for this installation: its `RSDKOWN3` namespace
journals code-volume ownership. Existing Image ownership roots are refused rather
than upgraded. Startup recovers recorded code effects before image pins and
qualifies the exact base/producer before advertising `workers.docker.zip=true`.
An image-only installation advertises false and rejects ZIP before fetching or
staging it. A failed/revoked installed source revokes backend readiness.

Each cold environment receives a unique daemon-local code volume. A never-started
RW loader transfers the original archive; the actual function receives only a
read-only `/var/task` mount, normalized root-owned code, and its own writable
noexec `/tmp`. Code preparation shares the environment's original start/invocation
deadline. Cancellation joins the original upload and physical consumer retirement
before releasing code-volume and environment reservations. Configure
`budgets.lambdaDockerCodeUploadSeconds` in YAML when tuning upload IO (default60,
range1..600 seconds); the cold deadline still caps it. Shutdown joins functions
and IO, then code volumes, image pins and the Docker wire.

`make test-lambda-provided-docker` selects the ordinary signed SDK/ASL/revision/
fault/cancellation cases and the exact-same-ZIP Linux Native verifier after the
compiled inventory check. It consumes the explicit fixture variables set by the
full gate; run `make test-lambda-docker` to provision and verify all66 resources.
Keep fixture files unchanged throughout a run. The verifier executes the same
original A ZIP bytes through the real Native provider and joins its child; it
uses an executable temporary mount only inside that separately owned verifier.
This does not change production Lambda's noexec policy.

Named snapshots include original ZIP bytes, revisions, publications, aliases and
terminal workflow history when the provided runtime is configured. Image-only
installations reject ZIP archives before startup ingress. Import validates the
complete private ZIP candidate before acquiring Image leases and publishes only
after final source/backend/deadline checks. Invalid metadata/code, unavailable
immutable content, a missing provided runtime or expiry preserves both committed
archive generations. No code volumes, containers, host extraction paths, warm
counters or live invocations are restored; a process restart starts with a fresh
runtime pool and does not replay function/workflow work.

The managed Lambda archive payload is capped at128MiB, with a separate16MiB
metadata limit. Original ZIP bytes are capped at50MiB per revision and64MiB total;
repeated latest/published copies count separately even when their live artifacts
share memory. The private restore candidate additionally caps expanded logical
file bytes at512MiB and materialized nodes at20,000, including three artifact
wrapper nodes per revision. Existing per-ZIP tree/expansion limits still apply.
The256-function/1024-revision limits remain. These are fixed safety profiles:
a deployment can be valid to invoke yet too large to save, in which case the
previous committed generation remains usable. Metadata limits apply globally
before an owned archive DTO is built; ZIP fields do not enlarge policy/layer or
other metadata allowances.

The provided base remains operator configuration, outside saved function code.
Restart requires a qualified immutable base for the saved ZIP architecture.
Changing that base deliberately changes the runtime environment while preserving
the original ZIP hash/size; archive metadata cannot select a base, registry,
network or credentials. Image archives continue requiring their exact saved
local image identities. `make test-lambda-zip-archive` runs the pure profile,
projection, codec, staging, candidate and clock controls;
`make test-lambda-provided-archive-docker` runs both ordinary archive resources
after fixture provisioning and the exact compiled manifest check.

Image callbacks and managed Pulumi IaC are functionally qualified on the retained
lanes. Server-in-container and the actual root release image are separately
qualified through D98/D99. Required arm64/reference/platform and complete release
acceptance remain separate under D100. The implementation ledger records final functional
qualification; the native-workflow verifier alone does not certify these Docker
archive resources.
See the [ordinary ZIP contract](../specs/rustack-workflows-20-lambda-runtime-design.md#68-ordinary-provided-runtime-installation-and-environment-consumption-d92)
and [managed archive contract](../specs/rustack-workflows-20-lambda-runtime-design.md#69-bounded-provided-runtime-zip-archives-and-mixed-candidate-restoration-d93)
and [verification status](../specs/rustack-workflows-93-review.md#d93-final-actual-qualification-and-exact-postconditions).

The management IO cases hold actual Create/Tag acknowledgments and owned DELETE
requests. Deleting a Pending deployment cannot release its quota while critical
image mutation/retirement remains owned. Admission transfers only when a live
artifact is consumed or its exact deployment commits. Whole transport loss can
revoke backend readiness and reject Invoke while diagnostics remain reachable;
restoring the test transport does not revive a failed owner. The test keeps that
transport alive through joined server cleanup, preserving any specific shutdown
failure. `make test-lambda-io-faults` runs the proxy's framing/lifecycle controls;
`test-lambda-io-faults-docker` runs the full ordinary startup/fault suite.

Additional cases keep the signed caller connected through actual Create/DELETE
acknowledgment expiration. Cleanup preserves the original handler exit failure.
The gate explicitly provisions the preserved immutable public Node baseline before
ordinary resources; `make lambda-pull-fixture-image` runs this bounded prerequisite
separately. An interrupted `ifMissing` pull preserves the old active revision while the fixture
retains and positively drains the real daemon stream. This explicitly connected
resource uses the pinned public Node source through its HTTPS port443 reference;
it refuses a preexisting extra cache reference and removes only its own verified
reference, preserving the original cache. `make lambda-negative-images` provisions
two genuinely incompatible platform artifacts and a separate Linux process that
never speaks Runtime API. They qualify preparation rejection and bootstrap timeout,
respectively. `make test-lambda-deadline-fixture` runs the bounded protocol, archive
and cleanup-receipt controls without executing Docker resources.

The additional Runtime HTTP resource sends unsupported methods, oversized header
values and declared bodies, a GET body and chunked trailers from a real consumed
custom RIC attempt, then requires the correct result and warm reuse. Separate
execution-capacity resources require actual8/32 permit ownership before429.
Physical-resource cases independently bind memory256MiB versus one environment,
and hold function then relay deletion to prove charges remain until full retirement.

## Enable trusted-host Lambda tasks

Build with the Lambda feature and select the backend explicitly. Override the
pure example's service/executor selection and require Lambda signature validation:

```bash
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  RUSTACK_CONFIG=examples/workflows/rustack.yaml \
  SERVICES=lambda,stepfunctions LAMBDA_EXECUTOR=native \
  LAMBDA_SKIP_SIGNATURE_VALIDATION=false \
  cargo run --locked -p rustack-cli --bin rustack \
    --no-default-features --features lambda,stepfunctions
```

Deploy a compatible ZIP/bootstrap through the Lambda API before starting a Task.
Native runs trusted host code, with a bootstrap matching the host OS and CPU; a
Linux deployment binary is not a macOS executable. Use the real fixture gates for
a reproducible computed result, failures, cancellation and SDK callbacks:

```bash
make test-stepfunctions-bridge-native
make test-stepfunctions-sdk
```

Direct function ARN Tasks return the function payload. The optimized
`arn:aws:states:::lambda:invoke` integration returns its Lambda envelope and
treats FunctionError as Task failure. `.waitForTaskToken` requires the installed
Lambda/callback service; SendTaskSuccess/Failure/Heartbeat act on secret tokens.
Nested `states:startExecution`, `.sync` and `.sync:2` are supported; waiting
parents release dispatch capacity. The [integration contract](../specs/rustack-workflows-40-integrations-design.md)
describes result shapes, scope/depth limits and cancellation ownership. Pure mode
rejects Lambda and callback definitions instead of inventing an execution port.

## Supported profile and limits

| Area | Installed behavior |
|---|---|
| Workflow type/query language | STANDARD and the documented JSONPath subset |
| States | Pass, Task, Choice, Wait, Succeed, Fail, Parallel, Inline Map |
| Data/recovery | InputPath/Parameters/ResultSelector/ResultPath/OutputPath, ordered Retry/Catch, bounded jitter/backoff and Task/execution deadlines |
| Intrinsics | Format, StringToJson, JsonToString, Array, ArrayLength, ArrayGetItem, ArrayContains, ArrayPartition, StringSplit, MathAdd, UUID, shallow JsonMerge |
| Lifecycle | CRUD/list/describe/tag definitions, empty unpublished-version inventory, Start/Stop/list/describe executions, captured-definition describe, execution history, Validate, installed callback APIs |
| Static ceilings | 1MiB definitions, 256KiB state payloads, 25000 events, Inline Map concurrency≤40 |
| Local budgets | Validated YAML `budgets.workflow*`; transition/history/frame/result/admission limits may terminate or reject work explicitly |

EXPRESS, JSONata/variables, Activities, Distributed Map/MapRun, redrive, arbitrary
AWS SDK integrations, published state-machine version/alias use and mutation, enabled
CloudWatch logging/tracing and customer-managed encryption are outside this
profile. Unknown fields/resources or unavailable integration profiles reject
explicitly. Do not infer implementation from the generated 37-operation model.
See the [API](../specs/rustack-workflows-10-api-design.md),
[ASL](../specs/rustack-workflows-30-asl-design.md) and
[configuration](../specs/rustack-workflows-60-operations-design.md#12-tunable-budgets)
contracts for exact grammar and ranges.

Definition updates affect new executions; existing ones keep their captured
revision. A RUNNING execution accepts a replay only with the same name and exact
original input. Closed names remain reserved for 90 days, even after shorter
history retention expires. An HTTP observer timing out does not imply the
accepted command or its physical cleanup was cancelled.

## Snapshots and shutdown

`--snapshot workflow-demo` loads before the listener opens and saves after a
proven shutdown cut. The root defaults to `.rustack/snapshots`; set
`RUSTACK_SNAPSHOT_DIR` to change it. One process owns a snapshot name at a time.
Use Ctrl-C/SIGINT or SIGTERM and wait for process exit. Restart with the same
configuration and snapshot name to read retained definitions, names, outcomes,
captured revisions and full history.

Shutdown stops intake, drains producers, gives workflows the configured grace
interval, cancels/joins remaining execution work, then joins downstream Lambda
and consumer work before saving. Live work stopped at this cut is archived
terminal. Restore does not resume tasks, timers, callbacks or processes and does
not replay Lambda side effects. Unexpected imported RUNNING records become
ABORTED/Rustack.ExecutionInterrupted. Old page/callback handles are invalid after
restart. SIGKILL can lose everything since the previous completed save: there is
no WAL. Unrecoverable corruption and incompatible/quota-rejected imports refuse startup.
Physical interrupted-publication recovery may select a previous valid generation;
domain/policy failure preserves both generations rather than silently rolling back.

Snapshot coverage is per service. Workflow resources/history and qualified Lambda
metadata/code are persisted; SQS messages and arbitrary external effects are not.
Image availability and cross-platform code execution remain separate requirements.

## Health, capabilities and qualification

```bash
curl --fail http://127.0.0.1:4566/_health/ready
curl --fail http://127.0.0.1:4566/_rustack/capabilities
```

`workflows` reports installed state types, integration flags and effective limits,
including `snapshot: "resources-and-history"`, `snapshotResume: false`,
`paginationEpoch: "fresh-on-restart"` and `dockerQualified: false`. Without a
selected Lambda port, `lambdaRequestResponse` and `callbacks` are false. Health
reads worker status independently of business admission; business quota rejection
does not by itself make a healthy service unready.

With Lambda installed, `workers.lambdaAdmission` shows actual
`acceptedInvocations` and `activeExecutions`, plus their limits128/32 and the
default per-function limit8. Active work includes backend pool waiting and
retained cleanup; it is not a count of running containers. The two counters are
independently sampled. Qualified Docker also reports `workers.docker.capacity`
with actual retained environment/memory/quarantine charges. These bounded views
contain no function names or request data and remain readable at work saturation.

```bash
make test-workflows-manifest  # Exact compiled ignored-test inventory preflight.
make verify-workflows-native  # Serial current-host workflow qualification.
```

The [reviewed inventory](../tests/workflows/required-suites.json) binds 26 resource
tests across Native bridge, registered SDK and typed HTTP SDK suites. Missing,
renamed, newly ignored or no-longer-ignored cases fail before resource execution.
The native verifier also runs model/codegen, compiler/runtime, archive,
persistence and performance checks. Full workspace build/Clippy/rustdoc and
audit/deny remain repository gates. This command certifies its native-workflow
scope; it does not certify the unfinished full image/platform release.

Current macOS measurements and raw provenance are in the
[performance review](./research/workflows-native-performance-review.md).
The [direct Linux measurement review](./research/workflows-linux-performance-review.md)
records the same signed SDK workloads inside a4-vCPU/8GiB native ARM VZ guest,
with p95=10.791ms, exact histories and joined process cleanup. To provision a new
dedicated test VM without activating its Docker context, then run the measurements:

```bash
make workflows-performance-linux-provision  # Refuses an existing profile.
make test-workflows-performance-linux       # Reuses the retained ARM guest.
```

The default profile is `rustack-workflows-performance-arm64`; select an explicit
profile with `WORKFLOWS_LINUX_PROFILE`. Both source and target mounts are read-only.
The VM remains running after the test. Four virtual CPUs do not prove four
dedicated physical CPUs: `referenceQualification` remains `pending`.
Required dedicated-reference Linux performance
and final platform lanes remain open in the
[implementation plan](../specs/rustack-workflows-90-impl-plan.md).
