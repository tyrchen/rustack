# Workflows 40: Lambda Tasks, Nested `.sync` and Callbacks

Status: implementation contract · Depends on: [API](./rustack-workflows-10-api-design.md), [Lambda runtime](./rustack-workflows-20-lambda-runtime-design.md), [ASL](./rustack-workflows-30-asl-design.md)

## 1. Internal ports and dependency direction

stepfunctions-core defines an object-safe LambdaTaskPort, a concrete channel-based ChildExecutionPort, and callback commands. The binary's `apps/rustack/src/stepfunctions_bridge.rs` injects a Lambda provider adapter. Core does not depend on lambda-core, an AWS SDK, HTTP loopback calls or Docker. Child execution uses a registry handle, avoiding a strong Arc cycle back to its own provider.

LambdaTaskPort logically exposes `start(validated command, deadline) → InvocationHandle`, with completion and `cancel_and_wait(deadline)`. The concrete channel-backed implementation uses synchronous bounded admission: the adapter retains the start future before returning TaskCall. The trait is object-safe without async-trait; concrete handle methods await results/cleanup. See [ASL§3.2](./rustack-workflows-30-asl-design.md#32-supervised-sequential-runtime-and-history-publication). ChildExecutionPort submits scoped Start/Subscribe/Cancel messages with parent references. Pure Pass workflows work without Lambda; compiling a Lambda integration without an installed adapter produces a dependency-disabled diagnostic.

Add a shared internal invocation-handle API to Lambda provider; HTTP Invoke delegates to it without changing wire behavior. Dropping the old response future cannot prove Docker cleanup. RAII may signal cancellation, but a supervisor must await cleanup; Drop cannot pretend to finish asynchronous IO.

```text
┌─ stepfunctions-core ───────────────────────────────────────────────┐
│ ExecutionActor → TaskAdapter enum                                 │
│    │              ├─ LambdaTaskPort (dyn) ───────────────────┐     │
│    │              ├─ ChildExecutionPort → registry actor     │     │
│    │              └─ CallbackRegistry: token + generation    │     │
│    └─ cancellation / generation validation                   │     │
└──────────────────────────────────────────────────────────────┼─────┘
┌─ binary composition root ────────────────────────────────────▼─────┐
│ StepFunctionsLambdaBridge → Lambda provider.start_invocation_owned│
│                            version resolution + same admission   │
└───────────────────────────────────────────────┬───────────────────┘
                                      Executor → actual runtime
```

### 1.1 Owned admission receipts and the binary Lambda bridge

Add `InvocationSubmission` to the shared Lambda API. `start_invocation_owned(reference, qualifier, payload, deadline, cancellation)` validates/resolves/captures a revision, then synchronously queues owned work and returns the submission before waiting for execution admission. An error before this return means no function call was queued. The submission retains its InvocationHandle independently of the admission result: `admitted(&mut self)` is cancellation-safe and `into_handle(self)` transfers the cleanup receipt even after admission rejection/expiry. Existing start_invocation/HTTP paths await admission through this receipt and retain their wire semantics. Dropping their old waiter still requests cancellation while the existing work supervisor owns cleanup.

InvocationHandle adds borrowed `wait_response`, `wait_cleanup` and `wait_settlement(deadline)` methods. The unbounded cleanup observer waits only on the retained supervisor state, not on IO; the bridge bounds its first observation and resumes the same handle after observer expiry. Consuming a response does not consume cleanup ownership. A second response read returns a typed error rather than repolling a completed oneshot. Settlement waiters can expire/be dropped and resume the same handle; Pending, Uncertain or closed channels never imply Clean. Retain the existing consuming wait/cancel_and_wait conveniences. Optional external cancellation is validated before/after artifact capture, before queueing and by InvocationContext before allocation/during execution. A bridge relay combines Step Functions cancellation and bridge shutdown; it stays alive through physical settlement. The bridge retains a pending start future until it returns a receipt or a prequeue error, even when its ASL caller has stopped.

`apps/rustack/src/stepfunctions_bridge.rs` implements LambdaTaskPort in a feature-gated public binary-support library. This is reusable composition code, not main-service registration. Construction requires an active Tokio runtime and an actual injected provider/scope. It returns an Arc port plus a supervisor controller. Synchronous start checks scope/deadline/accepting state, reserves bounded accepted-work capacity and queues before returning TaskCall. Defaults128 accepted calls, cleanup wait10s; validate1..1,024 and1..60s. The manager owns submissions, invocation worker JoinSet and per-call admission slots above worker lifetime. Never invoke Noop/echo as a production fallback. Construction rejects a disabled backend and mismatched provider partition/region/account; the future main composition root must install it only for an explicitly selected real backend.

The controller's shutdown uses one absolute deadline and retains the same manager JoinHandle after timeout or waiter cancellation. It stops admission and signals active workers before the Lambda provider itself quiesces. A worker panic retains its root slot and publishes no clean completion. Expired cleanup observation quarantines the business result while retaining the actual Lambda handle, QuarantinedTask receipt and admission slot. Resume the same cleanup observer: eventual Clean settles only the receipt, without publishing a second business result; terminal Uncertain/closed retains the unresolved graph. Drop transfers/drains ownership independently of public handles; an abort/runtime-loss guard preserves unresolved state as process-lifetime quarantine. Normal clean shutdown releases all reservations. Raw requests/responses, payloads, host errors and credentials never appear in Debug/logs; typed sources remain in redacted internal diagnostics.

At each attempt, decode direct Resource or optimized prepared Parameters, then call the same provider's owned API. Revalidate optimized object fields, FunctionName string≤256 bytes, optional Qualifier1..128 bytes and RequestResponse-only InvocationType after template evaluation. The bridge accepts only a plain function name or complete same-scope ARN; reject partial ARNs and name:qualifier forms after dynamic evaluation as well as static compilation. Let the provider's ordinary resolver validate name/ARN/qualifier existence/conflicts and capture the revision. Ordinary Lambda Invoke retains supported same-account partial ARNs but rejects mismatched partial-ARN accounts before local resolution. Omitted optimized Payload uses `{}` as the selected local default; explicit null/scalar/array/object payloads preserve their JSON type. Strings are JSON values, not pre-serialized input. No alias/revision pinning across retries and no adapter-added business retries.

On admission success, report Started once. Read response while retaining the Lambda handle; signal cancellation on ASL/manager stop and join settlement independently of the attempt deadline. Map business response under bounded retained CPU work. Direct success is the parsed JSON itself; optimized success is Payload, StatusCode=200, ExecutedVersion, SdkHttpMetadata.HttpStatusCode=200 and SdkResponseMetadata.RequestId from the actual admitted Lambda UUID. Do not synthesize cloud trace headers. Enforce256KiB on raw JSON and final envelope; quota failures are States.DataLimitExceeded, invalid JSON/UTF-8 or evaluated parameter shape are States.Runtime. Non200 executor responses are Lambda.ServiceException.

FunctionError alone selects the failure path; ordinary success keys named error/FunctionError/statusCode are business data. Parse a bounded errorType; empty/control-containing/over256-byte/reserved States.* values become Lambda.Unknown. Cause is valid raw error JSON, capped at32,768 bytes on a UTF-8 boundary with an explicit truncation marker; malformed error output uses a static safe cause. Runtime timeouts map Sandbox.Timedout, entrypoint/init failures Runtime.InvalidEntrypoint, runtime exits Runtime.ExitError, and known service exceptions Lambda.<code>. Unknown/IO/uncertain errors map Lambda.ServiceException with static public causes and retained internal sources. Core owns ASL States.Timeout and rejects late results. Only confirmed settlement calls TaskResponder.finish; an uncertain result uses quarantine and remains charged.

Qualify direct/optimized actual Native bootstrap execution, dynamic parameter rejection, FunctionError/Catch, retries, version/alias re-resolution, cancellation/timeout, function-owned Sandbox.Timedout and distinct metadata IDs before returning to image work. Cancellation/ASL-timeout fixtures must first observe a handler-written PID/request-ID marker after Runtime API event consumption, then assert that PID is absent after joined cleanup; Started alone proves admission, not handler execution. Controlled provider/executor fixtures separately exercise admission/cleanup races; they do not replace actual process execution. SDK/main-service and final real-image/platform acceptance remain required.

```text
┌─ Step Functions actor ──────────────────────────────────────────────┐
│ TaskCall ← synchronous bounded port.start(command)                  │
└────────────────────────────┬───────────────────────────────────────┘
                             ▼ manager owns queue / root slots / jobs
┌─ binary Lambda bridge worker ───────────────────────────────────────┐
│ retained start future → InvocationSubmission → admitted → Started  │
│                    │                 │                             │
│ cancellation relay └→ InvocationHandle.wait_response                │
│                         ↓ bounded result mapping                   │
│ retained handle.wait_settlement → Clean / owned quarantine         │
│                   TaskResponder.finish / QuarantinedTask            │
└────────────────────────────┬───────────────────────────────────────┘
                             ▼ same provider resolution and quotas
                 actual Native / Squib / ready Docker executor
```

## 2. Integration allowlist

| Resource | Pattern | Milestone | Successful Task result |
|---|---|---|---|
| `arn:<partition>:lambda:<region>:<account>:function:<name>[:qualifier]` | Direct Lambda RequestResponse | M1 | Function JSON payload itself |
| `arn:<partition>:states:::lambda:invoke` | Optimized RequestResponse | M1 | AWS envelope with Payload/StatusCode/ExecutedVersion/metadata |
| `...:states:::states:startExecution` | Await child start acceptance | M2 | ExecutionArn, StartDate; no wait for child termination |
| `...:states:::states:startExecution.sync` | Job wait | M2 | Child execution envelope; Output is a JSON string |
| `...:states:::states:startExecution.sync:2` | Job wait | M2 | Same envelope; Output is a JSON value |
| `...:states:::lambda:invoke.waitForTaskToken` | Callback | M3 | SendTaskSuccess JSON output |

Partition must match scope; initially region/account must also match. Reject `lambda:invoke.sync`, `aws-sdk:*`, other services' `.sync`, and direct-Lambda callback at validation. Match the entire service/action/pattern allowlist, never arbitrary resources ending in `.sync`. [AWS integration patterns](https://docs.aws.amazon.com/step-functions/latest/dg/integrate-optimized.html).

## 3. Lambda RequestResponse

Direct ARN passes Parameters, if supplied, otherwise effective input, as the function event; the result is the function payload. Optimized Parameters supports FunctionName, Payload, Qualifier and InvocationType. Omitted/RequestResponse is supported; Event/DryRun is rejected in this release to avoid confusing acceptance with function completion. FunctionName may be a supported name or ARN. Reject conflicting explicit and embedded qualifiers rather than silently choosing one. Dynamic values use the same Lambda resolver after evaluation.

Resolve aliases/latest at each attempt's invocation time and capture that Lambda revision for the call. Machine revision capture does not freeze all future Lambda aliases. Projects needing fixed code across retries use published version ARNs. Retry resolves alias/latest again; Rustack must not invent execution-wide alias pinning.

Optimized success contains parsed JSON Payload, StatusCode=200, ExecutedVersion, SdkHttpMetadata.HttpStatusCode and SdkResponseMetadata.RequestId. Only include meaningful bounded metadata; do not invent cloud tracing capabilities. Never JSON-encode Payload twice. Raw Lambda Invoke supports arbitrary Bytes, but ASL requires JSON: invalid UTF-8/JSON or oversized connector output fails explicitly without lossy conversion.

FunctionError causes Task failure even when Invoke returns HTTP200. Extract a bounded errorType and cause from the error payload; use Lambda.Unknown when no valid type exists. Do not infer failure from ordinary success payload keys such as error/statusCode. Adapter mappings are:

| Origin | ASL behavior |
|---|---|
| Lambda reserved/global concurrency rejection | Lambda.TooManyRequestsException |
| Known Lambda service exception | `Lambda.<AWS exception code>`, without host internals |
| Function `/error` | User errorType; reserved `States.*` names become Lambda.Unknown |
| Function's own timeout | Sandbox.Timedout or supported runtime-specific error |
| Initialization/entrypoint failure | Runtime.InvalidEntrypoint or appropriate Runtime.* class |
| Step Functions Task deadline | States.Timeout; cancel handle and reject late completion |
| Docker transport failure with uncertain result | Lambda.ServiceException; internal deliveryUnknown diagnostic; no automatic replay |

P0 fixtures pin exact exception/event details. Only explicit ASL Retry repeats business calls; adapters must not add hidden invocation retries. Retrying deliveryUnknown may repeat external side effects, so retain attempt IDs for diagnosis.

## 4. Child workflows and `.sync`

Parameters supports StateMachineArn, Input and Name. Normalize omitted/null Input to `{}`. An Input string is already serialized JSON: validate and preserve its bytes rather than encoding it again. Compact-serialize other JSON values. Passing the serialized string `null` therefore differs from a null Parameters value. Users may place `AWS_STEP_FUNCTIONS_STARTED_BY_EXECUTION_ID.$: $$.Execution.Id` inside child Input. Do not inject fields into business payloads. Generate an independent UUID name when absent and store parent linkage internally.

RequestResponse waits only for acceptance. Such a detached child is not cancelled when its parent subsequently fails/stops, but remains owned by the global supervisor. `.sync`/`.sync:2` establish an owned relationship. Subscribe atomically with Start or use a versioned subscribe-then-read protocol; a fast child must not terminate before its parent can observe completion. Deliver termination once. Use internal events, not HTTP polling or EventBridge rules.

```text
Parent actor              Registry actor               Child actor
 │1. StartChild + subscribe      │                           │
 │─────────────────────────────▶│2. name/revision/admission   │
 │                              │──────────────────────────▶│
 │◀── executionArn + lease ─────│                           │
 │3. release dispatch slot       │                           │
 │   await child terminal        │                           │
 │                              │◀── terminal/read view ────│
 │◀──4. child result ───────────│                           │
 │   .sync: Output string        │                           │
 │   .sync:2: Output JSON        │                           │
 │ Stop/timeout ────────────────▶│5. cancel owned child ─────▶│
 │◀── cleanup settlement ───────│◀── child cleanup/terminal ─│
```

Use the optimized integration's PascalCase ExecutionArn/StateMachineArn/Name/Status/Input/Output/StartDate/StopDate envelope. Dates in optimized child-start and child-completion envelopes are integer epoch milliseconds, unlike HTTP API fractional epoch seconds. Do not reuse the camelCase HTTP DescribeExecution serializer. `.sync` retains Input/Output as JSON strings; `.sync:2` decodes both Input and Output into JSON values. Include InputDetails/OutputDetails.Included where applicable. The P0 oracle memo pins the upstream AWS-validated evidence and its historical-date limitation. [AWS nested workflows](https://docs.aws.amazon.com/step-functions/latest/dg/connect-stepfunctions.html).

FAILED/TIMED_OUT/ABORTED children fail the parent Task with States.TaskFailed and bounded child ARN/error/status cause. A child's timeout is not the parent's own States.Timeout. Parent Task timeout, Stop or sibling-branch failure requests cancellation of an owned child. Single terminal settlement resolves concurrent completion; successful children and external side effects are never rolled back.

The default 128 open-execution limit includes parents and children; nesting depth≤16. **A waiting parent holds no dispatch slot or Lambda permit**, preventing parents from exhausting all capacity required by children. If open-execution admission is exhausted, return ExecutionLimitExceeded rather than queue a child forever. Recursive workflows are allowed within depth/history/duration limits. Retry creates another child attempt; a user-supplied fixed Name still obeys normal idempotency/name-retention rules.

### 4.1 Example: wait for a real child workflow

The child can be the image-Lambda machine from the PRD. The parent definition is:

```json
{
  "StartAt": "RunChild",
  "States": {
    "RunChild": {
      "Type": "Task",
      "Resource": "arn:aws:states:::states:startExecution.sync:2",
      "Parameters": {
        "StateMachineArn": "arn:aws:states:us-east-1:000000000000:stateMachine:ImageChild",
        "Input.$": "$"
      },
      "TimeoutSeconds": 30,
      "OutputPath": "$.Output",
      "End": true
    }
  }
}
```

The parent must remain RUNNING until the child's container handler finishes. A failed child cannot produce a successful parent output.

## 5. Callback and heartbeat, M3

Support optimized Lambda `.waitForTaskToken` only. Before dispatch, generate a 256-bit CSPRNG token, register its hash/scope/execution/frame/generation, and then evaluate Parameters so `$$.Task.Token` exists. Invoke Lambda synchronously. After Lambda succeeds, await callback; the Lambda return payload is not the Task output. Lambda failure revokes the token and follows ordinary failure handling.

A callback can arrive before the Lambda invocation returns. Store it provisionally in the same owner, commit it after successful Lambda return, or discard it if Lambda fails. A fast callback is neither lost nor allowed to hide invocation failure.

Task timeout includes invocation and callback waiting. Heartbeat timing starts only after successful synchronous Lambda return. SendTaskHeartbeat extends the heartbeat window, not the absolute Task deadline. Retry/timeout revokes old tokens and creates a fresh token for the next attempt. Without a heartbeat field, do not invent a short heartbeat timeout. [AWS Lambda callback timing](https://docs.aws.amazon.com/step-functions/latest/dg/connect-lambda.html).

SendTaskSuccess.output is a JSON string≤256 KiB. Failure.error/cause are≤256/32,768 bytes. Token input is≤2,048 bytes. Malformed, unknown or cross-scope tokens return InvalidToken; known revoked/completed/timed-out callback tasks return TaskTimedOut. TaskDoesNotExist is reserved for the nonexistent-activity API case, outside this callback-only profile. Retain bounded token tombstones for execution retention to preserve classification.

Treat tokens as credentials. Infrastructure tracing/Debug/metrics/causes never contain raw tokens; the registry stores hashes. Before history persistence, redact values injected from Task.Token in materialized Parameters and mark execution-data truncation. Advertise this local history difference; never save raw tokens and hope a later logger redacts them. This does not promise scanning arbitrary business data for secrets. Use constant-time comparison where token material is compared and a production security-grade random source.

## 6. Rust engineering contract and cross-references

Follow AGENTS.md Error/Async/Type/Safety/Serialization: typed thiserror mappings, documented dyn object safety, supervisor-owned handles, bounded IO/channel budgets, validated private scope/ARN/token types, redacted Debug and camelCase internal envelopes. Tests cover absent adapters, HTTP200 function errors, alias updates, child subscription races, early callbacks, cancellation and deadlines. Tracing records IDs; internal calls avoid HTTP serialization and reparsing. Public APIs need runnable SDK examples and Errors documentation.

- ← [API](./rustack-workflows-10-api-design.md), [Lambda](./rustack-workflows-20-lambda-runtime-design.md), [ASL](./rustack-workflows-30-asl-design.md)
- → [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md)
- ↔ [key decisions](./rustack-workflows-99-decisions.md)
