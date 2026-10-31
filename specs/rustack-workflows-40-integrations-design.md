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

`apps/rustack/src/stepfunctions_bridge.rs` implements LambdaTaskPort in a feature-gated public binary-support library. This is reusable composition code, not main-service registration. Construction requires an active Tokio runtime and an actual injected provider/scope. It returns an Arc port plus a supervisor controller. Synchronous start checks scope/deadline/accepting state, reserves bounded accepted-work capacity and queues before returning TaskCall. Defaults128 accepted calls, cleanup wait10s; validate1..1,024 and1..60s. The manager owns submissions, invocation worker JoinSet and per-call admission slots above worker lifetime. Never invoke Noop/echo as a production fallback. Construction rejects a disabled backend and mismatched provider partition/region/account; the main composition root installs it only for an explicitly selected real backend.

The controller's shutdown uses one absolute deadline and retains the same manager JoinHandle after timeout or waiter cancellation. It stops admission and signals active workers before the Lambda provider itself quiesces. A worker panic retains its root slot and publishes no clean completion. Expired cleanup observation quarantines the business result while retaining the actual Lambda handle, QuarantinedTask receipt and admission slot. Resume the same cleanup observer: eventual Clean settles only the receipt, without publishing a second business result; terminal Uncertain/closed retains the unresolved graph. Drop transfers/drains ownership independently of public handles; an abort/runtime-loss guard preserves unresolved state as process-lifetime quarantine. Normal clean shutdown releases all reservations. Raw requests/responses, payloads, host errors and credentials never appear in Debug/logs; typed sources remain in redacted internal diagnostics.

At each attempt, decode direct Resource or optimized prepared Parameters, then call the same provider's owned API. Revalidate optimized object fields, FunctionName string≤256 bytes, optional Qualifier1..128 bytes and RequestResponse-only InvocationType after template evaluation. The bridge accepts only a plain function name or complete same-scope ARN; reject partial ARNs and name:qualifier forms after dynamic evaluation as well as static compilation. Let the provider's ordinary resolver validate name/ARN/qualifier existence/conflicts and capture the revision. Ordinary Lambda Invoke retains supported same-account partial ARNs but rejects mismatched partial-ARN accounts before local resolution. Omitted optimized Payload uses `{}` as the selected local default; explicit null/scalar/array/object payloads preserve their JSON type. Strings are JSON values, not pre-serialized input. No alias/revision pinning across retries and no adapter-added business retries.

On admission success, report Started once. Read response while retaining the Lambda handle; signal cancellation on ASL/manager stop and join settlement independently of the attempt deadline. Map business response under bounded retained CPU work. Direct success is the parsed JSON itself; optimized success is Payload, StatusCode=200, ExecutedVersion, SdkHttpMetadata.HttpStatusCode=200 and SdkResponseMetadata.RequestId from the actual admitted Lambda UUID. Do not synthesize cloud trace headers. Enforce256KiB on raw JSON and final envelope; quota failures are States.DataLimitExceeded, invalid JSON/UTF-8 or evaluated parameter shape are States.Runtime. Non200 executor responses are Lambda.ServiceException.

FunctionError alone selects the failure path; ordinary success keys named error/FunctionError/statusCode are business data. Parse a bounded errorType; empty/control-containing/over256-byte/reserved States.* values become Lambda.Unknown. Cause is valid raw error JSON, capped at32,768 bytes on a UTF-8 boundary with an explicit truncation marker; malformed error output uses a static safe cause. Runtime timeouts map Sandbox.Timedout, entrypoint/init failures Runtime.InvalidEntrypoint, runtime exits Runtime.ExitError, and known service exceptions Lambda.<code>. Unknown/IO/uncertain errors map Lambda.ServiceException with static public causes and retained internal sources. Core owns ASL States.Timeout and rejects late results. Only confirmed settlement calls TaskResponder.finish; an uncertain result uses quarantine and remains charged.

Qualify direct/optimized actual Native bootstrap execution, dynamic parameter rejection, FunctionError/Catch, retries, version/alias re-resolution, cancellation/timeout, function-owned Sandbox.Timedout and distinct metadata IDs before returning to image work. Cancellation/ASL-timeout fixtures must first observe a handler-written PID/request-ID marker after Runtime API event consumption, then assert that PID is absent after joined cleanup; Started alone proves admission, not handler execution. Controlled provider/executor fixtures separately exercise admission/cleanup races; they do not replace actual process execution. The registered main service and ordinary Rust SDK/boto3 qualification now exercise this actual Native path; final real-image/platform and later integration acceptance remain required.

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

### 4.1 Atomic child-execution registry port

Before ASL resource admission is enabled, implement the concrete `ChildExecutionPort` returned by `Registry::child_port()`. It holds a Weak reference to the registry admission state, never an Arc cycle through execution owners. `ChildStartParameters::parse(scope, payload)` immediately validates the exact StateMachineArn/Input/Name fields. Reuse scoped ARN/name newtypes; reject other fields and nonstring Name/ARN. Omitted/null Input becomes `{}`; string Input is validated serialized JSON with original bytes retained, other JSON values are compact encoded. Invalid dynamic parameters map to States.Runtime; input/envelope quota violations map to States.DataLimitExceeded.

`ExecutionSnapshot::child_parent()` yields an opaque capability retaining that exact charged parent record. Registry admission verifies Arc identity against its own retained record, fresh RUNNING state and scope; same-scope records from another registry cannot authorize a child. Accepted commands capture this capability, a deadline clamped to the parent's captured absolute deadline, typed RequestResponse/Sync/Sync2 pattern, cancellation control and independent admission lease. Every execution records a bounded immutable lineage (root depth0, children at most16); owned replay edges also enforce the longest active dependency path≤16. No business Input mutation supplies lineage. A fresh child extends the parent lineage; idempotent RUNNING replay preserves its original identity/lineage. Public names and ordinary Start continue using the existing atomic registration/idempotency rules.

The registry owns a separate bounded child-request lane and retained pending request before entropy work. The root retains its own admission state until joined shutdown, so losing external registry handles does not orphan execution-side requests. Its live child-call ceiling equals the runtime open-execution ceiling; it uses a separate semaphore, never the ordinary command-admission lane or a Task/Lambda dispatch permit. Requests reject saturation rather than waiting forever. At commit, recheck cancellation, absolute deadline, parent identity/liveness and depth. Reserve relation identity and reject self or active dependency-graph cycles before creating an owned edge. Active owned relations form a bounded directed acyclic graph; idempotent replay may join the same RUNNING child, and each owned relation can request cancellation of that selected execution. A graph-cycle attempt is a local States.Runtime safety failure; depth/call/open saturation maps to StepFunctions.ExecutionLimitExceeded. Failed starts publish neither a new record nor a name claim.

Synchronous command transfer returns `ChildSubmission` before awaiting admission. `admitted(&mut self, deadline)` retains its receiver on observer expiry/cancellation and caches the one accepted child or typed rejection, with fresh pre/post observer deadline fences even when the reply was already ready; `into_execution(self)` transfers the unique cancellation owner. Drop/cancel before commit prevents issuance. Drop/cancel after owned commit signals the registry; a lost reply still drops the returned child capability and requests owned cleanup. RequestResponse detaches at acceptance, so later caller/parent loss cannot cancel that child. `ChildExecution` keeps snapshot/terminal/cleanup observations independent and cancellation-safe; observer deadlines never release unsettled ownership. `result()` is pure bounded envelope mapping for an embedding CPU owner, and requires an observed terminal result for Sync/Sync2.

Start and subscription are one non-awaiting registry transaction: install the owned relation before returning a captured watch handle, including when the child already terminated. The registry retains the relation/admission lease until child cleanup is Clean. Notify wakes cancellation promptly; monotonic maintenance checks parent termination and child deadlines without HTTP polling. Propagate cancellation through all owned descendant relations in one bounded graph traversal; detached edges are excluded. Alternate ready ordinary/child intake, allowing at most one child turn before a ready ordinary command. While any UUID worker is retained, continue selecting cancellation notifications, monotonic maintenance and close alongside its join; closed intake must promptly stop roots and reject unissued queued children even before that worker returns. Root shutdown closes both intake lanes, rejects queued child requests, stops/joins all roots through the existing runtime drain, and retains pending/unknown state on failure or observer loss.

Use integer epoch milliseconds from captured accepted/stopped dates for optimized envelopes. Include PascalCase execution/machine ARN, Name/Status, Input/Output and Included details for successful Sync variants. Map unsuccessful child terminal status to States.TaskFailed with bounded JSON ARN/status/error/cause; never flatten a child's timeout into the parent's States.Timeout. Record only actual values; never invent cloud metadata. Validate exact string-versus-JSON/null/raw-input mapping, quotas, dates, foreign/closed parents, replay/collision/cycle/depth/call/open limits, fast completion, lost/cancelled/expired admission observers, detached survival, nested cancellation, retained cleanup and actual Native child execution before using this port in ASL actors. The Native cancel scenario uses a300s relation deadline,3600s parent Wait,900s Lambda timeout and180s handler gate around a30s business scenario, so unrelated clocks cannot masquerade as explicit cancellation. Probe the consumed PID before global cleanup; retain/reap the exact probe process on timeout and exercise that forced-reap path independently. This dependency alone does not enable `.sync` resources or advertise nested service capability.

```text
┌─ caller / parent observer ────────────────────────────────────────────┐
│ exact snapshot capability → ChildExecutionPort (Weak admission)        │
│ ChildSubmission → ChildExecution → result / independent cleanup views │
└───────────────────┬───────────────────────────────────▲───────────────┘
                    │ bounded transfer                  │ captured watch
┌─ retained registry root ───────────────────────────────┴───────────────┐
│ ordinary lane ⇄ fair intake ⇄ child lane + independent call quota      │
│ retained pending → UUID worker join ⇄ cancel / deadline / close        │
│ fresh identity/depth/cycle/control checks → atomic Start + owned edge  │
│ Notify + maintenance → cancel owned DAG → retain edge until Clean      │
└───────────────────┬───────────────────────────────────────────────────┘
                    ▼ runtime owns every root, with no wait dispatch slot
              child actor → actual Lambda port → retained IO settlement
RequestResponse detaches at acceptance; unknown joins retain ownership.
```

### 4.2 ASL binding, registration gate and retained child Task worker

Trusted compilation enables child resources independently of Lambda support. Match only the three exact partition-scoped startExecution resources in §2. Validate literal StateMachineArn/Name and serialized Input immediately; dynamic fields pass the same validated domain parser after Parameters evaluation. Unknown parameters and normalized literal/dynamic collisions reject. Callback resources remain unavailable. Recursively distinguish Lambda dependency from child dependency across Parallel/Map scopes.

Registry admission routes machines containing child Tasks through the structured owner, including sequential one-frame machines. Public unregistered supervisor starts reject child-dependent artifacts. Registered starts allocate a one-shot parent-capability gate before spawning; the registry inserts the exact charged record and name claim before publishing its ChildParent. The actor awaits that gate before evaluating work, while still honoring Stop/execution timeout. A closed gate fails without issuing work. Ordinary sequential Lambda/pure machines retain their established execution path.

Each accepted child Task retains a frame-only resource lease in the supervisor ledger before spawning a protocol worker. It acquires no Task dispatch permit, including while parsing, submitting, waiting for completion or awaiting cleanup. Pending child Tasks can pass a Lambda Task waiting for a dispatch permit. Accepted child work is bounded by frame/open/child-lane capacity; payload parsing and envelope serialization additionally use retained bounded CPU jobs. The actor joins workers and result watchers before releasing resources. An unexpected worker loss closes progress/settlement, cancels owned relations and preserves actual child/CPU receipts in charged process quarantine; channel closure never proves cleanup.

The worker retains the same ChildSubmission through caller cancellation, Task timeout and delayed admission. After cancellation it requests cancellation and still resolves acceptance to determine issuance. Explicit rejection before issuance is clean; a lost admission owner is uncertain. RequestResponse detaches on registry acceptance and returns its start envelope without waiting for child business/cleanup. Owned patterns emit Started then Submitted only after real acceptance, observe terminal business outcome, cancel on parent Task control, and publish the mapped terminal business result once before awaiting actual physical cleanup to confirm settlement. A later Task deadline cannot replace an already observed business result merely because cleanup remains pending; frame retirement/recovery still waits for cleanup. Observation expiry never drops a receipt. Cleanup waiting observes in-memory retained execution state rather than external IO; quarantine remains charged until proven Clean or permanently retained after owner loss.

```text
Registry                  Structured owner            Child Task worker
  │ allocate gate ───────────▶│                              │
  │ commit record + name      │ wait gate / Stop / deadline   │
  │ publish ChildParent ─────▶│                              │
  │                          │ ledger(frame, no dispatch)    │
  │                          │ spawn retained worker ───────▶│
  │◀─────────────────────────┼── parse(CPU), child port.start│
  │ atomic acceptance ───────┼─────────────────────────────▶│
  │                          │◀── Started, Submitted         │
  │ child terminal + cleanup ┼─────────────────────────────▶│
  │                          │◀── mapped business result     │
  │                          │    retain frame / child       │
  │                          │◀── physical cleanup: Clean    │
  │                          │ join worker/watcher, retire   │
  │◀── Stop/timeout ──────────┼── cancel, retain same receipt │
```

Child Task history uses TaskScheduled/TaskStarted/TaskSucceeded/TaskFailed/TaskTimedOut/TaskStartFailed with resourceType `states` and the exact resource action (`startExecution`, `startExecution.sync`, `startExecution.sync:2`). Sync patterns additionally produce TaskSubmitted between Started and completion, with the actual ExecutionArn/StartDate start envelope as output. RequestResponse has no Submitted event. Per-frame predecessor chains remain causal; includeExecutionData=false removes Submitted output and outputDetails alongside existing Task data. No synthetic child ARN or invented Lambda event is emitted.

Acceptance requires real registry execution for all three patterns; string/JSON output distinction, fast completion, detached child survival, owned Stop/Task timeout cleanup, errors/capacity/depth and no-dispatch deadlock tests. The service capability remains false until actual registered Native and both ordinary SDK nested suites qualify this binding. Docker remains deferred.

### 4.3 Example: wait for a real child workflow

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

Treat tokens as credentials. Infrastructure tracing/Debug/metrics/causes never contain raw tokens; the registry stores hashes. Before history persistence, redact values injected from Task.Token in materialized Parameters; mark truncation only on event details that actually support it, as specified in§5.2. Advertise this local history difference; never save raw tokens and hope a later logger redacts them. This does not promise scanning arbitrary business data for secrets. Use constant-time comparison where token material is compared and a production security-grade random source.

### 5.1 Owned callback registry foundation

Implement a concrete scoped `CallbackRegistry` and unique `CallbackSupervisor` before ASL callback resources are enabled. The registry is a bounded actor, independent of HTTP, Lambda and Docker; its consuming workflow owner supplies an exact captured `ExecutionSnapshot`, opaque `FrameCallId`, Task deadline and optional heartbeat duration in `CallbackAttempt`. Validate same-scope execution identity and current RUNNING parent state at registration and callback mutation. Clamp the Task deadline to the captured parent deadline. Minting is a trusted internal operation, never a public SendTask API. The consuming engine must additionally supply its current frame/attempt and revoke its unique task receipt on retirement; the foundation does not claim ASL admission, context injection or provenance redaction.

`register(attempt)` synchronously reserves live-token, future-tombstone and command/reply capacity before queuing a retained mint request. Reserve closed classification at mint admission, so later timeout/cancellation cannot lose a known token when tombstone capacity is exhausted. Defaults are live4,096, retained100,000 (active plus reserved/closed), commands1,024, entropy workers8 and retention24h; validate positive documented ranges and retained≥live. Active records release live capacity on terminal callback state; their digest/attempt/parent classification reservation remains until the parent execution terminates and execution retention expires. Never evict live entries to admit another token. Once a terminal result is published, drop the table's outcome sender and invocation receiver: only caller-owned watch readers may retain business payloads/failure causes. Tombstones retain lightweight identity/classification, never completed output data. A duplicate captured execution/frame/generation cannot mint a second token while its classification record is retained. Each record has one removable timer, so arbitrarily many heartbeats cannot accumulate timer nodes.

The actor retains bounded blocking entropy joins while continuing to process already issued token commands, control notifications, timers and close. Use production `getrandom` for32 random bytes, encode exactly64 lowercase hexadecimal bytes and transfer the token once in a secret wrapper. Zeroize transient entropy buffers and the token on Drop. Store only SHA-256 digests in actor indexes; digest equality uses constant-time comparison. The plaintext exists only in the retained entropy result/registration reply, the unique `CallbackTask` and explicitly exposed outbound/context material, never in table keys, infrastructure Debug, tracing, error text or metrics. `CallbackToken::from_str` enforces the input's2,048-byte cap and exact local opaque format; malformed/unknown tokens produce InvalidToken. Caller scope is trusted request context, never token-supplied data; foreign scope always produces InvalidToken, including for a locally known closed digest. Tokens are not serializable by default. Use [secrecy0.10.3](https://docs.rs/secrecy/0.10.3/secrecy/)'s explicit secret exposure and zeroize re-export, plus the workspace's existing subtle2.6/hex0.4 cohort; verify current upstream APIs and dependency gates.

The unique `CallbackTask` holds its token, monotonic control and watch receipt. `invocation_complete(Result<(),TaskFailure>)` is one-shot; Drop/revoke is an idempotent cancellation signal with independent Notify, never a best-effort queue send. Both mint receipts and task observers survive observation timeouts; dropping a mint receipt revokes even before admission. Dropping an API acknowledgment waiter never undoes its accepted callback action. Cache late-ready replies before enforcing a fresh post-await observer deadline, preserving the same acceptance/error for later observation. A fresh clock/parent/control fence precedes every state mutation. An execution stopped or completed while a callback is pending revokes that task; known closed token classification remains TaskTimedOut.

```text
Execution owner                 Callback actor                 API caller
  │ reserve + register ───────────▶│                              │
  │                               │ bounded retained CSPRNG join  │
  │◀── unique token/task receipt ─│ store digest + attempt only    │
  │ invoke Lambda                 │◀── SendTaskSuccess/Failure ────│
  │                               │ Invoking: provisional result  │
  │                               │──────── accepted / closed ───▶│
  │ invoke success ──────────────▶│ commit provisional OR Waiting  │
  │ invoke failure ──────────────▶│ discard provisional; fail Task │
  │◀── watch terminal state ──────│                              │
  │ Drop / revoke ───────────────▶│ tombstone; independent Notify  │
  │                               │◀── heartbeat while Waiting ──│
  │                               │ replace one heartbeat timer  │
  │                               │ deadline / root close: revoke│
```

While Invoking, accept at most one provisional success/failure; subsequent completion or heartbeat on that consumed token returns TaskTimedOut. Before any provisional completion, heartbeat is accepted but does not start/reset a clock. Successful Lambda completion uses the captured return timestamp to compute heartbeat deadline=min(observed+interval, absolute Task deadline). A provisional result commits only when its actor-receipt timestamp precedes that deadline; otherwise the Task expires. With no provisional result, start Waiting only if the derived deadline is still in the future. Lambda failure wins over a stored provisional callback and publishes InvocationFailed. While Waiting, a completion commits once; heartbeat replaces only its heartbeat timer and cannot extend the absolute Task deadline. A heartbeat at or after either deadline fails TaskTimedOut. No heartbeat setting means no heartbeat timer. Preserve distinct Task-deadline and heartbeat-expiry reasons for the consuming engine's ASL mapping. On simultaneous readiness the actor checks fresh absolute/heartbeat deadline and revocation/parent liveness before invocation or external completion; already committed terminal state is immutable. Accepted provisional completion closes the external token but cannot bypass the still-running Lambda invocation's absolute deadline.

`SendTaskSuccess` preparation parses bounded JSON without double encoding. Failure preparation preserves omitted/empty error/cause and validates256/32,768-byte ceilings. Valid command acceptance returns a bounded resumable acknowledgment; classification is InvalidToken for unknown/foreign/malformed, TaskTimedOut for known consumed/revoked/expired, and explicit Capacity/Unavailable/Deadline for infrastructure admission/observation. No TaskDoesNotExist fallback, synthetic success, hidden retry or raw token in diagnostics. Root close rejects queued unissued work, revokes active entries, joins accepted entropy work and supports resumed shutdown observation. Root panic/runtime loss marks issued receipts unavailable, requests revocation and conservatively retains unresolved jobs/permits in process quarantine; closed channels never imply successful callback completion.

Foundation acceptance covers early/late success and failure, Lambda failure after early success, heartbeat before/after activation and exact boundaries, absolute deadline, revoked/duplicate/foreign/unknown tokens, parent Stop, old attempt reuse, same-attempt mint collision, capacity and retention recovery, saturated command lane with independent revoke, observer loss/late replies, entropy delay/panic and resumable root shutdown. Debug/table/token-source scans must prove no plaintext storage or formatting. ASL `.waitForTaskToken`, injected-parameter history redaction, public SendTask routing, actual handler callbacks and service capability remain consuming P4 work; this foundation alone must not advertise callbacks.

### 5.2 Single-evaluation callback Parameters and history provenance

Before the ASL actor can admit callback resources, add `DataFlow::prepare_callback_with_source(original, base_context, token, source) -> CallbackParameters` as the pure bounded token/materialization boundary. The caller must already own an actually registered `CallbackTask` and pass its `CallbackToken`; this helper does not mint or start a function. It evaluates InputPath and Parameters once with a transient Task.Token context. `CallbackParameters::outbound()` is the actual invocation payload; `history()` returns a distinct, immutable `CallbackHistory` that has no public unchecked constructor. `CallbackHistory::payload()` exposes only the prepared history JSON and `redacted()` declares the local policy. Both types have redacted Debug and no implicit credential-bearing serialization. Preparation failures return no partial pair and must cause the consuming owner to revoke the same task before retry/retirement.

The supplied base context must be an object without the reserved Task member. Reject stale/forged Task context rather than overwriting it. Clone its bounded validated tree once, inject only `{ "Task": { "Token": <registered secret> } }`, and keep this context local to preparation. Do not mutate the base, store Task.Token in the driver, carry it into the next state/retry, or inject it into business Input automatically. The sensitive context storage clears the injected token String with zeroize when its final Arc drops, including error/unwind and shared-context clones; ordinary contexts retain their existing cheap Arc sharing. Enforce the existing4MiB/68-level/context-member limits, with checked size accounting for the fixed credential envelope, before publishing this sensitive context. Do not serialize the plaintext-bearing context just to measure it. Outbound payloads intentionally contain requested token material; they remain bounded/redacted immutable Payloads and must never be passed to the callback history projector.

Compiled paths classify provenance structurally, independent of raw string contents. A context-root selection is token-sensitive when its compiled member/wildcard sequence may select Task.Token or an ancestor containing it: `$$`, `$$.Task`, `$$.Task.Token`, bracket-equivalent names, `$$[*].Token`, `$$.Task[*]`, `$$[*][*]`. Context siblings such as State/Execution, nonmatching/case-distinct keys and data-root paths are not intrinsically sensitive. Indices/slices cannot traverse the reserved object-member address. An expression is conservatively sensitive if any eagerly evaluated argument is sensitive, including Format, JSON/string conversion, arrays, length, item selection and merge; replace the complete dynamic expression result in history, never search its serialized value for the token. This intentionally redacts a boolean/count or an enclosing object when computed from sensitive context, even when it no longer contains plaintext. It does not scan arbitrary business data for secrets.

InputPath provenance must follow the effective input into Parameters. If InputPath selects sensitive context, every data-root dynamic reference to that effective input is conservatively sensitive too. Literal template members remain literal. If Parameters is omitted, redact the complete sensitive effective input. A null InputPath produces an ordinary empty input. Each dynamic expression executes exactly once under the existing shared work/output budget and caller UUID source; build actual and history projections from that same prepared tree. Untainted dynamic values, UUID results, business fields and literals remain identical in both projections. Sensitive dynamic nodes serialize to JSON null only in history. Any resulting actual or history payload must independently satisfy the existing256KiB/depth/member limits before the pair is returned, including pathological redaction expansion for tiny intrinsic results. Failure never silently truncates an otherwise invalid payload. Original input and base context remain immutable.

The schema qualification resolves a spec ambiguity: [TaskScheduledEventDetails](https://docs.aws.amazon.com/step-functions/latest/apireference/API_TaskScheduledEventDetails.html) has required parameters JSON but no execution-data-details/truncated member, as confirmed by the pinned generated model. Therefore its parameters contain the null-redacted projection, without a fabricated AWS field or wrapper. Internal `CallbackHistory::redacted()` and the eventual advertised local history policy identify the difference. Callback invocation submission history will omit the ignored synchronous Lambda business Payload and mark `TaskSubmitted.outputDetails.truncated=true` where that official field exists. This does not mark unrelated state input as truncated. The consuming history adapter must take CallbackHistory explicitly; callback resource/API capability stays disabled until registered token preparation, invocation/result ownership and actual handler/SDK history scans are qualified together. Result-phase and later-state contexts never gain this transient credential.

```text
registered CallbackTask.token ──────┐
original Input + immutable context ┼──▶ scoped sensitive context
                                  │        │ cleared on last Drop
InputPath (provenance bit) ────────┼────────▼─────────────────────┐
compiled Parameters               │ one bounded evaluation tree  │
                                  │ dynamic values + provenance  │
                                  └────────────────┬──────────────┘
                                      ┌────────────┴─────────────┐
                                      ▼                          ▼
                            CallbackParameters.outbound   CallbackHistory
                            actual requested token        tainted nodes:null
                                      │                          │
                                Lambda owner              history projector
```

Acceptance covers all supported member/bracket/wildcard/ancestor paths, effective-input propagation, nested templates/arrays/intrinsics, static business values equal to a token, shared source/budget accounting, exactly-once UUID acquisition, unchanged input/context, rejection of existing Task context, sensitive context headroom/zeroization, independent history limits and failure without partial output. Serialize the history projection through the actual generated TaskScheduled model to prove no injected plaintext or invented wire field. These pure preparation proofs do not establish ASL callback admission, history persistence or SendTask service acceptance.

### 5.3 Observed synchronous Lambda return and independent cleanup

Before the ASL callback owner can use the real Lambda port, separate the unique invocation-report capability from CallbackTask. `CallbackTask::take_invocation_reporter()` transfers a noncloneable `CallbackInvocationReporter` exactly once; the Task keeps its token, revocation control, invocation observation and callback-result watch. Its existing direct `invocation_complete` remains available only while it owns the reporter. Neither path may report twice or report after delegation, revocation, closure or the absolute Task deadline. Dropping an unreported reporter revokes the attempt through independent control; dropping a reported reporter preserves the pending callback. Task Drop still revokes regardless of reporter ownership.

A reporter captures validated scope, exact opaque FrameCallId, registration-commit time and absolute Task deadline. `complete_at(result, observed)` accepts only a nonfuture observation at/after registration, then applies fresh closed/revoked/deadline fences before a single watch publication. Its successful result is typed `CallbackInvocationMetadata`, with a real version4 request UUID and actual `$LATEST` or canonical positive numeric executed version. Do not accept arbitrary strings/payload objects as response metadata, invent a version, or include the synchronous function business body. Expose immutable `CallbackInvocation` observations (captured return time, optional metadata or bounded failure) separately from callback business outcome and from physical cleanup. Direct foundation reports may omit metadata; the eventual ASL owner must require real metadata before projecting invocation submission.

The callback registry consumes the same captured report time for heartbeat start. Retain one actor-receipt timestamp beside provisional completion, so a callback received at/after observed+interval cannot bypass expiry when reporting is delayed. A genuinely pre-return callback or a callback received within the window remains eligible even if mapping/report publication occurs later. Heartbeat commands received while still Invoking remain the §5.1 no-op, including the observation-to-report gap; they never extend the captured window. Only commands linearized after successful report activation reset the heartbeat timer. An observation arriving after its heartbeat window cannot extend that window. Early callback success/failure remains provisional until a valid successful report; invocation failure discards it. Reporter watches preserve late-ready success/error/owner-loss observation fences and never duplicate invocation work. Terminal table records continue dropping their outcome sender and invocation receiver; caller-owned observers alone may retain report data. Metadata is bounded by its typed schema rather than an unrestricted256KiB response payload.

Add `LambdaTaskCommand::observed_callback(machine, dispatch, call, reporter)` for the internal optimized request-response call. Validate the exact optimized Lambda resource, machine scope, call/attempt identity and unused live reporter before transfer; clamp the invocation deadline to the captured callback deadline. A rejected command drops/revokes its unique reporter. This is an internal invocation observation mode, not an enabled ASL callback definition/profile. The consuming callback owner will materialize the actual CallbackParameters, commit CallbackHistory, and then hand the same registered reporter to this command. Ordinary direct and optimized RequestResponse commands retain their established payload/settlement behavior.

The binary bridge consumes this mode explicitly. Before any actual issuance, validate dynamic optimized parameters as usual. For accepted calls, publish Started only after real provider admission. Capture the monotonic return time immediately when the real InvocationHandle yields its response, before retained blocking result mapping or cleanup. On HTTP200 without FunctionError, ignore the function body completely (including non-JSON or ASL-oversized bodies already bounded by Lambda), produce only actual typed invocation metadata, and report success at the captured time. FunctionError, admission/executor failure and invalid trusted metadata report a bounded invocation failure; an early callback can never hide them. Mapping uses the same retained worker/cancellation/InvocationHandle ownership. No business invocation retry is added.

The current Lambda provider publishes InvocationHandle responses after executor cleanup/settlement. This slice observes that provider-return boundary, not the earlier handler Runtime API POST; do not claim independently delayed backend cleanup on successful Native responses. Publish the invocation report before the bridge waits for its independent physical-settlement receipt. The low-level TaskCall still owns its independent cleanup receipt and returns only safe metadata in observed mode. Cleanup expiry/uncertainty must not rewrite an already reported business return or fabricate Clean: retain the actual handle, report/settlement receipts and admission charge through resumed cleanup or quarantine. Never drop a blocking mapping join when cancellation arrives. Caller/manager cancellation must relay while mapping and cleanup are pending. Ordinary Lambda behavior remains unchanged. The later callback owner releases its workflow dispatch permit only after invocation retirement, while retaining the callback frame/token through external completion.

```text
CallbackTask -- transfer once --> InvocationReporter --> LambdaTaskCommand
      │ own token/revoke/watch                           │ real bridge
      │                                                  ▼
      │                                      retained provider admission
      │                                                  │ Started
      │                                                  ▼
      │                                      actual response + captured time
      │                                                  │ bounded mapping
      │◀── invocation metadata/failure + observed ─────────┤
      │    commit provisional / start heartbeat           │
      │                                                  ▼
      │ independent callback wait               retained physical cleanup
      │                                                  │
      └─ callback result / revoke              TaskCall Clean / quarantine
```

Acceptance requires exact delegation/report/time/scope/attempt invariants, old report rejection, report observation before held bridge settlement delivery, heartbeat anchored before delayed mapping, drop/revocation and resumed observers, and no terminal-payload retention regression. Qualify the real binary bridge with actual consumed Native handlers: early and late callbacks, failure after early success, stop/reap, ignored synchronous bodies, actual request/version identity and no token in metadata/Debug. Existing M1/M2 SDK regressions remain required. ASL callback scheduling/history, public SendTask APIs, actual handler-originated SDK callbacks and service capability remain separately gated until consuming integration; Docker remains deferred.

## 6. Rust engineering contract and cross-references

Follow AGENTS.md Error/Async/Type/Safety/Serialization: typed thiserror mappings, documented dyn object safety, supervisor-owned handles, bounded IO/channel budgets, validated private scope/ARN/token types, redacted Debug and camelCase internal envelopes. Tests cover absent adapters, HTTP200 function errors, alias updates, child subscription races, early callbacks, cancellation and deadlines. Tracing records IDs; internal calls avoid HTTP serialization and reparsing. Public APIs need runnable SDK examples and Errors documentation.

- ← [API](./rustack-workflows-10-api-design.md), [Lambda](./rustack-workflows-20-lambda-runtime-design.md), [ASL](./rustack-workflows-30-asl-design.md)
- → [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md)
- ↔ [key decisions](./rustack-workflows-99-decisions.md)

### 5.4 Deferred ASL callback instruction and pure effect boundary

The ordinary Task driver evaluates Parameters before it emits TaskDispatch. Callback Tasks require the opposite ownership order: assign the attempt/frame/deadline first, register the token against that identity, then evaluate Parameters exactly once with the registered credential. Do not insert placeholder JSON, mint a token inside the pure driver, defer ordinary Lambda templates, or temporarily inject Task.Token into retained driver context.

Add an independent trusted `AslCompiler::with_callback_support(bool)` capability, default false. With Lambda enabled and callback support explicitly selected, accept only same-partition `lambda:invoke.waitForTaskToken`; ordinary Lambda/child resources still reject heartbeat fields. Store callback heartbeat as a validated TaskTimeout source inside a distinct `Integration::LambdaCallback` variant. Static and dynamic heartbeat seconds and CallbackAttempt registration use one shared positive1..99,999,999 profile; literal/path forms are mutually exclusive. A configured heartbeat must be strictly smaller than the configured Task timeout: check two literals during compilation and all resolved combinations before emitting an effect. For validation when Task timeout is omitted, use the AWS Task documentation's99,999,999-second default ceiling; an explicit heartbeat equal to that ceiling is invalid. Resolve dynamic values from original state input/context, before InputPath/Parameters. Omitted Task timeout still retains the existing operator/execution deadline cap; its validation ceiling does not create a new timer or compare heartbeat with remaining parent duration. See [ASL Task timeouts](https://states-language.net/spec.html#task-state) for mutual exclusion, positive integers and the strict interval relation, and [AWS Task fields](https://docs.aws.amazon.com/step-functions/latest/dg/state-task.html) for the99,999,999-second omitted-timeout validation ceiling (verified2026-10-26). The latter agrees with the existing supported numeric profile; it does not adopt the ASL text's different default timer.

A callback-dependent root or nested lexical scope emits `DriverStep::Callback(CallbackDispatch)`, never ordinary Dispatch. The immutable effect retains its exact machine/state, fresh attempt, original input, credential-free State/Execution context, absolute Task/execution minimum deadline, optional resolved heartbeat and optional explicit timeout seconds for history. It performs no entropy, registration, invocation or template evaluation. Generation/RetryCount/deadline and stale/duplicate completion semantics reuse the existing Phase::Calling protocol. FrameScheduler charges/tracks/cancels/retires this effect exactly like an ordinary Task call, including equal frame-local generation numbers in different branches. The effect is cloneable only as immutable instruction data; token/report ownership stays unique in the registry and consuming worker.

`CallbackDispatch::prepare_with_source(token, source)` evaluates the effect's captured DataFlow once using §5.2, returning a materialized TaskDispatch plus distinct CallbackHistory. Bind preparation to the effect's captured machine, not a caller-supplied state ID/artifact. Keep original input/context immutable; errors return no partial invocation/history output and preserve typed sources. The async consumer remains responsible for fresh control/deadline/attempt checks before and after bounded CPU work, matching the exact registered token to this attempt and revoking on preparation failure. A cloned effect cannot authorize duplicate business issuance.

The generic history scheduled helper rejects callback resources. A separate typed scheduled_callback helper accepts only CallbackHistory, preserves explicit resolved timeout/heartbeat seconds (omitted fields remain absent), and uses the official TaskScheduled DTO without invented truncation fields. Callback TaskSubmitted includes actual bounded metadata and outputDetails.truncated=true because the synchronous Lambda body is deliberately omitted. TaskStarted/TaskSucceeded/TaskFailed/TaskTimedOut retain the callback resource name and causal frame linkage; CallbackDispatch output preparation never finishes the ASL Task. Only the selected external callback result passes through the existing ResultSelector/ResultPath/OutputPath and Retry/Catch engine.

```text
┌─ Pure frame driver ───────────────────────────────────────────────────┐
│ resolve timeout/heartbeat → fresh generation → Phase::Calling         │
│ emit CallbackDispatch(machine, state, original, context, deadline)    │
└─────────────────────────────────┬────────────────────────────────────┘
                                  ▼ exact opaque FrameCallId
┌─ Consuming async owner (subsequent binding) ───────────────────────────┐
│ register actual parent/attempt → unique token/report owner            │
│ bounded prepare once → actual TaskDispatch + typed CallbackHistory    │
│ commit/ack Scheduled history → issue observed Lambda command          │
│ real return → Submitted metadata → external result → driver.complete │
│ Stop/timeout/retry → revoke + cancel + retain joined cleanup/charges   │
└──────────────────────────────────────────────────────────────────────┘
```

Stage this pure compiler/effect/history dependency independently of activation: sequential ExecutionDriver rejects callback-dependent artifacts; current ExecutionSupervisor (including start_structured) and Registry Create/Update must also reject them until the registered async consumer is installed. API preparation and main callbacks capability remain false. Extend the internal observed Lambda command/bridge allowlist to the explicit callback integration only when its unique reporter is present; no ordinary response-mode callback invocation is permitted. Qualify compiler errors and nested capability propagation, no pre-registration UUID/template work, exact captured context/clock/deadline behavior, single evaluation/provenance, dynamic interval errors, Retry/Catch, stale completion, structured frame identity/retirement/cancellation, generated event wire fields and all current Native/SDK regressions. The subsequent parent registration/history acknowledgment/retained callback worker, public SendTask and handler-originated SDK acceptance are still required before M3 is complete; Docker stays deferred.

### 5.5 Registered asynchronous callback owner

Install the actual callback registry into a fresh ExecutionSupervisor through a checked, immutable composition method. It must reject reconfiguration after work/shared ownership exists and require the same trusted scope when Registry composition binds it. Registry exposes callback support from its installed runtime; it admits compiled callback-dependent definitions only with that binding. Subsequent API activation must derive compilation from that capability together with installed SendTask routing. This library binding precedes main/API capability activation and handler-originated SDK acceptance; no test-only or disabled backend may advertise production readiness.

Extend the existing atomic parent-registration gate to transfer the exact ExecutionSnapshot after record/name insertion, converting it to ChildParent only for child job calls. Callback-dependent machines use the structured owner even without Parallel/Map. Unregistered start/start_structured paths continue rejecting callbacks, even when a registry port was injected. A failed Start cannot publish a token, Scheduled event or business invocation before the exact snapshot commits. Keep the gate independent of ordinary registry command and Lambda dispatch permits.

For each CallbackDispatch, retain the exact frame lineage in the supervisor ledger before starting its joined callback worker. Record call state/deadline through a separate instruction descriptor; never manufacture TaskDispatch parameters for bookkeeping. The worker owns mint reply, CallbackTask, retained preparation JoinHandle, actual Lambda TaskCall, physical dispatch permit and preparation-byte charge. A Drop/panic guard requests token/call cancellation and retains unknown receipts/charges. The actor retains and joins callback jobs alongside child jobs and resource watchers; channel loss never certifies cleanup. A retirement that changes graph readiness invalidates the parked timer, so parent recovery cannot sleep past completed sibling cleanup.

Charge callbacks to the same global/local intermediate-result byte pools as fanout. Before mint, reserve MAX_PAYLOAD_BYTES plus64KiB protocol/failure allowance in the frame's immutable call lease; keep it until resource retirement and consumed result notifications, including provisional callback/result queues. Queue business/completion metadata together with an owned lease, retain terminal child charges while their notification is Waiting/Queued, and reconcile only after the returned update is consumed and downstream aggregation is charged. Before bounded preparation CPU work, reserve2*MAX_PAYLOAD_BYTES plus256 bytes for the outbound/history pair, then shrink that charge to actual encoded pair sizes plus256 bytes. Hold it through actual invocation cleanup, because command/bridge copies may still retain outbound credential data. Byte reservation failure is an explicit unrecoverable Rustack.ResultLimitExceeded resource failure; token/registry admission capacity uses Rustack.CallbackLimitExceeded. Both reject without hidden retry, never an unbounded allocation or a wait holding scarce dispatch capacity. Shared CPU permits bound temporary materialization; no token/callback waiter holds a CPU permit after its job joins.

Register CallbackAttempt from the committed snapshot, exact FrameCallId, effect deadline and heartbeat. Resolve the same retained mint reply through cancellation/deadline rather than dropping accepted entropy work. Prepare the captured effect once under CPU/byte admission with a short-lived secret-wrapped token copy. Preserve template errors as typed evaluation-origin failures with the same classification/recovery as eager Task Parameters; do not accidentally make States.Runtime catchable or lose the source by mapping every error to Lambda.ServiceException. Retained CPU cancellation joins the same job before disposal.

After preparation, send a bounded actor message containing FrameCallId, typed CallbackHistory and a unique acknowledgment sender. The actor checks fresh Stop/deadline/current generation, commits the official Scheduled event on its causal frame cursor, publishes the coherent view and acknowledges whether issuance remains authorized. The worker must observe positive acknowledgment and recheck its controls before acquiring/issuing real Lambda work. History failure, closed acknowledgment, Stop, timeout or superseded generation revokes the token; there is no unscheduled business call.

Acquire a physical dispatch permit only for actual Lambda issuance and cleanup. Build the explicit observed command with the same registered reporter and materialized TaskDispatch. Forward actual Started once; successful actual invocation report publishes TaskSubmitted metadata with supported truncation. Invocation failure overrides provisional external completion. The external callback result alone produces TaskSucceeded/TaskFailed; heartbeat expiry maps States.HeartbeatTimeout with TaskTimedOut history, absolute Task timeout remains States.Timeout. Use the captured report timestamp for activation (§5.3), not cleanup observation. Waiting for external callback must not retain a physical dispatch permit after confirmed Lambda Clean. Keep the frame/token/result charge until the callback Task retires. A single-dispatch runtime must therefore run another workflow's Lambda while a prior callback waits externally.

Drive actual invocation progress/settlement and callback completion independently. Publish the selected business result before late physical cleanup when appropriate, using the existing completion/settlement receipt protocol. Pending/Quarantined cleanup cannot rewrite an observed report or release its physical permit; continue the same join or retain quarantine. A callback-root failure must not prevent an independently provable physical cleanup from releasing its dispatch/preparation charge; retain only the remaining unknown token/frame/result receipts. Stop/Task timeout/sibling cancellation revokes the token, cancels the actual call, joins accepted preparation/mint work, waits for registry terminal revocation so provisional payload is no longer table-owned, and retires only after proven cleanup. Unexpected revocation of an unreported live invocation is typed ownership loss, not a recoverable timeout. Closed callback/root/CPU/port ownership is a typed fatal condition; request cancellation and continue provable cleanup, retaining unknown receipts when proof fails. A joined preparation-only CPU panic proves no invocation was issued and may clean its revoked token, while a callback/port worker panic retains its exact unknown receipts. Never poll an already consumed failed JoinHandle again. Preserve the first diagnostic source through later cleanup errors. Token tombstone classification remains available after worker retirement.

```text
Registry owner       Structured actor       Retained callback worker       Lambda port
  │ commit snapshot ───────▶│                         │                         │
  │                        │ frame/result lease ────▶│ register + resolve mint │
  │                        │                         │ bounded retained CPU    │
  │                        │◀── typed Scheduled+ack ─│ one preparation         │
  │                        │ commit/publish ── ack ─▶│                         │
  │                        │                         │ dispatch permit + call ─▶│
  │                        │◀── Started/Submitted ──│◀── actual report ───────│
  │                        │                         │◀── independent Clean ──│
  │                        │                         │ release physical permit │
  │                        │                         │ await external callback │
  │                        │◀── selected result ────│                         │
  │                        │ driver/recovery/retire │ settle after all joins  │
  │                        │ Stop/timeout ──────────▶│ revoke + cancel + join  │
  │                        │ root/worker panic ─────▶│ retain unknown leases   │
```

Qualification requires real registered ASL executions with a controlled observed port, then actual Native consumption: early/late success and external failure, function failure after provisional success, captured heartbeat expiry/reset, timeout, Stop, Retry fresh token/old-token rejection, nested Parallel/Map cancellation, exact causal history and pre-issuance history failure, single-dispatch progress while callbacks wait, result/CPU/token quotas, resumed shutdown, worker/source failures and credential scans. Default unbound runtimes/registries remain rejecting. Keep public SendTask routing, ordinary handler-originated SDK callbacks, persistence and final image/platform gates explicitly incomplete until their consuming acceptance runs. Docker remains deferred.

### 5.6 Real image SDK callbacks and independent Event worker (D96)

Implementation/acceptance contract for task4.3/C01–C03 and5.3c, after D95/L10. The registered callback owner, public APIs and Native SDK consumers already exist; this slice qualifies their actual Docker composition. Preserve the existing token, timer, admission, history and cleanup contracts. Do not add another state-machine engine, callback registry, authentication scheme or implied broad SDK capability.

**Source and independent worker.** Build a separately named callback image on the exact D94/D95 pinned Node24 base. The D96 bounded unstarted source reader verifies actual client-lambda, client-sfn and client-dynamodb3.1105.0 package metadata, removes its exact read-only/network-none container and preserves all eleven preceding sources. This is metadata, not callback execution. The image exports dispatcher and worker handlers; two Image functions select them through CMD. Constructors use default endpoint/credential providers, maxAttempts1 and D95's exact qualified SDK context. No endpoint/credential constructor override, npm installation, host credential lookup or arbitrary host mount is introduced.

For late callback modes, dispatcher A invokes worker B with the real Lambda SDK, InvocationType=Event. Require actual HTTP202 acceptance, not worker completion. Forward the injected token only in that bounded invocation payload; keep it out of DynamoDB control rows. A then returns an intentionally ignored synchronous body. B remains in its own real active invocation while awaiting a caller-controlled gate, and sends the Step Functions callback after A's successful TaskSubmitted observation. The Lambda Event owner retains the accepted B work independently. This uses the documented [asynchronous queue/202 contract](https://docs.aws.amazon.com/lambda/latest/dg/invocation-async.html) and [callback heartbeat activation after Lambda return](https://docs.aws.amazon.com/step-functions/latest/dg/connect-lambda.html), checked2026-10-30. No work needs to continue in A after its handler returns.

```text
 caller / ordinary SDK       workflow + callback owner       Image A       Event owner       Image B
          │                           │                         │               │               │
  1 Start ├──────────────────────────►│ mint scoped token       │               │               │
          │                           ├─ real observed Invoke ─►│               │               │
          │                           │                         ├─ SDK Event ──►│ retain B work │
          │                           │                         │◄─ HTTP202 ────┤──────────────►│
          │                           │◄─ A returns / Clean ────┤               │               │
          │◄─ causal TaskSubmitted ───┤ release dispatch slot    │               │               │
  2 gate ├───────────── local DynamoDB safe control row ──────────────────────────────────────►│
          │                           │◄──────── signed SendTaskSuccess/Failure/Heartbeat ─────┤
          │◄─ callback Task outcome ──┤                         │               │◄─ B completes─┤
          │                           │                         │               │ join accepted │
 Stop ───►│ revoke token + cancel A   │                         │               │ B independent │
          │ confirm A pair absent; release B; B proves TaskTimedOut; join B; save/scan; audit all │
```

**Explicit fixture profile.** Reuse the ordinary Docker target and four-feature cohort. Add only its callback profile to the explicit SDK/service opt-in, keeping strict Lambda/Step Functions/DynamoDB signatures and the same valid server aliases. Use two environments, function memory256MiB each plus two64MiB relay charges, with lambdaDockerMemoryBytes805306368. D95's two SDK profiles retain capacity1. Set init20s/function timeout45s and one workflow dispatch slot; callback tokens2/tombstones64/commands8/workers1 match the existing Native constrained profile. Enable named snapshots for this profile. SDK bodies and gates share an original35s handler budget, individual sends at most4s, and an explicit abort/client/timer cleanup path. No transport/business retry or detached task is added.

**Safe control channel.** Host SDKs create one local DynamoDB table before function deployment. Its bounded rows contain only nonce, role, actual request UUID, attempt0/1, phase, arithmetic, SDK versions, counters and SHA256(token). Gate rows contain caller-selected booleans and bind the case nonce plus attempt number, so a consumed first-attempt return grant cannot release a retry. Never store raw tokens in rows, function environment, output, causes or diagnostics. Validate payload shape, identifiers and collection/string/number bounds before SDK IO. A/B use the D95 endpoint/pair rather than another control listener. Cap the fixture at64 rows/2KiB per row,64 reported token hashes and16KiB report bytes. Retain the original token only in invocation-local memory and A's bounded previous-attempt memory for the retry oracle; compare token material with constant-time Node crypto. All error messages are fixed, and raw SDK errors/input are never printed.

**C01 causal cases.** Use the existing independent callback case catalog for expected statuses/errors/arithmetic. Early success/failure calls the actual SDK while A remains gated: require RUNNING, Scheduled with redacted token and no Submitted before releasing A. An early success followed by A's fixed FunctionError must fail as CallbackInvocationFailure and retire A's pair. Late success uses B, requires A's actual request/version metadata in Submitted before releasing B, and returns callback-computed42 rather than A's ignored body. While B waits and the callback owns no physical dispatch permit, another workflow must compute42 through the already warm A using the one dispatch slot. Keep a slot available for both A/B; do not allocate an unrelated third function just for this check.

**C02/C03 timer and generation cases.** Retry CallbackRetry once; require two A request UUIDs in the same warm environment, retry counts0/1, distinct token hashes and actual old-token Heartbeat→TaskTimedOut before the new callback. For heartbeat reset, A sends an Invoking heartbeat and remains gated longer than a3s interval while the parent stays RUNNING. Release A, observe Submitted, then enable B heartbeats; keep the parent RUNNING beyond another complete interval with at least four acknowledged heartbeats before success. Absolute Task timeout8s remains fixed while B continually heartbeats; require States.Timeout, then B's success attempt→TaskTimedOut. A no-heartbeat mode yields States.HeartbeatTimeout after successful A return, followed by the same precise stale-token rejection. Default Task timeout is60s/execution timeout120s; prewarm A/B before timed cases to isolate the timer contract from image initialization.

For Stop, A accepts B's Event and remains in its own invocation. Observe both active pairs, Stop the parent with error Stopped, and require ABORTED/no Submitted. Prove A's exact pair is absent while B's pair remains; then release B and require its callback→TaskTimedOut before joining it. B is explicitly independent Event work and must not be counted as an A cleanup leak. Forged tokens produce InvalidToken, and duplicates/completed tokens produce TaskTimedOut through the actual image SDK. Retain existing Native/public wrong-scope tests; do not infer multi-account image authentication from this single-account fixture.

**Evidence, archives and ownership.** Every case records safe actual A/B UUIDs and token hashes. The host independently checks contiguous history IDs/previous IDs, expected TaskSubmitted count and real request/version identity, null redacted Scheduled token, supported body-omission marker and exact terminal result/error. Each image also checks that its token is absent from actual SDK GetExecutionHistory. The host scans histories/logs and the saved archive independently by hashing bounded64-hex candidates and comparing with reported token hashes; it never needs a raw token. Limit the private saved generation to128 filesystem entries/64 regular files/depth8/4096-byte paths, compressed4MiB per file/16MiB total and plain16MiB per file/64MiB total; logs are at most1MiB. Cap candidate hashes at65,536 and known hashes at64, rejecting malformed hash inputs before IO. Scan zstd-decoded bytes for compressed entries and raw bytes for plain manifest data, with a single30s observation deadline and joined accepted decoder jobs even on expiry. Reject links/special files; never follow an archive path outside the private generation. Verify the two functions through the existing typed Lambda metadata reader and require every reported terminal execution ARN in the actual Step Functions metadata, so an empty/unrelated archive cannot pass. Include a deliberate known-token leak control to prove detection and cap/invalid-file controls.

The hash observer must test every overlapping64-byte lowercase-hex window, including a token embedded in a longer hex run. Scan a capped complete decoded buffer or carry the final63 bytes across decoder chunks; never miss a split token or double-charge an already scanned window. Charge one aggregate65,536-candidate budget across all history, log and archive sources for that resource, carrying the host history count into the post-stop audit. Exhaustion rejects the run rather than stopping with a clean result. Controls include shifted/embedded tokens, a decoder-chunk split and exact/over aggregate caps across multiple sources.

Set/read back reserved concurrency1 for both A/B before prewarming or accepting an Event. Keep functions, state machines and safe control rows until confirmed graceful snapshot save, so archive scans inspect real retained resources. Before save, positively observe every expected B terminal marker and then a successful reserved=1 worker probe; a probe alone cannot prove a queued Event was delivered. Reuse the existing image_sdk Server/child owner with an async post-stop audit hook; default Node/SDK consumers use a no-op wrapper. Preserve the original PID cutoff, always-join failure paths, server stop/audit/log checks and retained evidence on any failure. The hook may inspect a saved archive only after successful stop; no duplicated Runner or lifecycle implementation is allowed.

**Delivery.** Freeze two exact ordinary resources before execution: test_should_execute_real_image_callbacks_before_and_after_lambda_response and test_should_enforce_image_callback_retry_heartbeat_timeouts_and_stop. Ordinary35→37/all68→70/eight suites. Split the catalog's four C01 modes and five timer/retry/Stop modes between them, with actual forged/duplicate checks alongside. Add explicit pinned image/fixture/focused Make targets and include the image in lambda-docker-fixtures; all full lanes preserve twelve actual source IDs. Run both focused/current full lanes, all existing required resources, standard/strict/Native/security/text gates and independent review with exact owned cleanup. IaC/server-in-container/native-reference/Desktop/rootless/final5.4 remain open. No image callback completion is inferred from D95's metadata or S3/DynamoDB acceptance.
