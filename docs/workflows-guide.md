# Step Functions and Lambda workflows

Rustack runs STANDARD workflows through its own ASL engine. The opt-in
`stepfunctions` source feature supports a bounded JSONPath profile, all eight
state types, retries/timeouts, Inline Map/Parallel, nested workflows and Lambda
callbacks. Real Lambda execution is currently qualified through the explicit
trusted-host Native backend. Production Docker image startup, image ZIP execution,
IaC and final platform qualification remain unfinished; selecting the ordinary
CLI Docker backend still fails before listening.

## Run the pure example

Run the commands from the repository root, using Cargo and the pinned Rust toolchain. The qualification commands require a Unix host
(Linux or macOS); recorded current-host performance evidence is on macOS/aarch64. The feature is absent from the default build;
adding `SERVICES=stepfunctions` to an unchanged default binary cannot enable it.

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
| Lifecycle | CRUD/list/describe/tag definitions, Start/Stop/list/describe executions, captured-definition describe, execution history, Validate, installed callback APIs |
| Static ceilings | 1MiB definitions, 256KiB state payloads, 25000 events, Inline Map concurrency≤40 |
| Local budgets | Validated YAML `budgets.workflow*`; transition/history/frame/result/admission limits may terminate or reject work explicitly |

EXPRESS, JSONata/variables, Activities, Distributed Map/MapRun, redrive, arbitrary
AWS SDK integrations, state-machine versions/aliases/publication, enabled
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
Required reference Linux performance, production image startup/ZIP, handler SDK
networking, Pulumi lifecycle and final platform lanes remain open in the
[implementation plan](../specs/rustack-workflows-90-impl-plan.md).
