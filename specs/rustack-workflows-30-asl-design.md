# Workflows 30: ASL Compiler and State Machine Engine

Status: implementation contract · Depends on: [API/data model](./rustack-workflows-10-api-design.md)

## 1. Compile before execution

Create, Update and Validate share AslCompiler and produce immutable CompiledMachine objects. Never repeatedly parse definitions or resolve string Next targets during execution. Reject duplicate JSON keys and enforce byte/depth limits before constructing serde_json::Value. Business payloads are dynamic JSON; state definitions must become typed tagged enums rather than arbitrary maps interpreted throughout the engine.

Pipeline: bounded bytes → strict AST → field/state validation → scoped graph validation → Path/Intrinsic AST → integration resolution → indexed IR. StateId is a checked internal index. Next/Catch/Choice targets stay within their machine scope: no branch jumps to a parent or sibling machine. Loops are valid; do not require a DAG. Require a reachable terminal state; unreachable states are warnings.

Reject unsupported fields, JSONata, Assign, Credentials, dynamic Resource, Distributed Map and unavailable integrations during creation. Diagnostics carry JSON Pointer location, severity, stable Rustack code and bounded message without copying definition/input into logs. QueryLanguage may be omitted or explicitly JSONPath.

## 2. Supported ASL profile

The [ASL specification](https://states-language.net/spec.html) supplies language terminology. The table below is the release contract; supporting eight state types does not imply support for every recent language extension.

| State | Supported fields beyond Type/Comment and applicable InputPath/OutputPath | Constraints |
|---|---|---|
| Pass | Result, Parameters, ResultPath, Next/End | Result and Parameters are mutually exclusive locally; no Retry/Catch |
| Task | Resource, Parameters, ResultSelector, ResultPath, Retry/Catch, TimeoutSeconds/Path, HeartbeatSeconds/Path, Next/End | Exact integration allowlist; heartbeat supported for callback only |
| Choice | Choices, Default | No top-level Next/End or ResultPath; first matching rule wins |
| Wait | Seconds, SecondsPath, Timestamp, TimestampPath, Next/End | Exactly one wait source |
| Succeed | InputPath, OutputPath | No Next/End or Retry/Catch |
| Fail | Error/Cause, ErrorPath/CausePath | Each literal/path pair is exclusive; no input/output transforms or Next/End |
| Parallel | Branches, Parameters, ResultSelector, ResultPath, Retry/Catch, Next/End | At least one independent branch machine |
| Map, INLINE | ItemsPath, ItemSelector, ItemProcessor, MaxConcurrency/Path, ResultSelector, ResultPath, Retry/Catch, Next/End | Normalize legacy Iterator/Parameters; reject simultaneous legacy/current equivalents |

States supporting Next/End require exactly one, with End=true. Choice Next exists only inside rules. ProcessorConfig only permits Mode=INLINE. MaxConcurrency and MaxConcurrencyPath are exclusive. Compile-time constraints fail compilation; dynamically selected values are validated on entry and never replaced with defaults after an error.

### 2.1 Paths

Use winnow for an explicit ASL subset rather than treating a general JSONPath crate as the AWS contract. Support `$` and `$$`, `.identifier`, `['quoted-key']` with bounded quote/backslash escaping, nonnegative `[index]`, object/array `[*]`, nonnegative `[start:end]`, and consecutive wildcards such as `$[*][*]`. Reject filters/scripts, recursive descent, unions, negative indices/steps and dot-escape syntax at creation. ReferencePath permits only root/member/index; ResultPath must target `$`, never `$$`.

Return Missing, Single or Selection, distinguishing JSON null from no match. Wildcard/slice selections remain arrays even with one match. Array ordering is stable; object wildcard uses deterministic key order as a local behavior, not an AWS object-order guarantee. Reference lookup failures do not panic or become null.

### 2.2 Data processing

```text
┌─ immutable raw state input S ─────────────────────────────────────┐
│ InputPath(default $, null→{}) → effective input I                 │
│                    ▼                                              │
│ Parameters template(I,$$), or I when absent                        │
│                    ▼                                              │
│ Task / Pass / Parallel / Map produces raw result R                 │
│                    ▼                                              │
│ ResultSelector(R,$$), or R when absent                             │
│                    │                                              │
│ S ────────────────▶ ResultPath(default $, null→preserve S) → M      │
│                                                            │      │
│ OutputPath(default $, null→{}) ◀─────────────────────────────┘      │
│                    ▼ next state's raw input                      │
└───────────────────────────────────────────────────────────────────┘
       Every expansion/merge checks the 256 KiB payload budget.
```

ResultPath merges into original S, not the Parameters result. Missing object members may create intermediate objects; writes through scalar/null/wrong container types fail, and arrays do not grow to arbitrary indices. ResultPath=null discards the result. Parameters/ResultSelector recursively process object/array `.$` fields. Dynamic fields require valid Path/Intrinsic strings; collision after removing `.$` is a compile error. ResultSelector `$` refers to raw result, not state input.

Bounded writers enforce final UTF-8 JSON size without allocating unbounded output first. Connector results are size-checked before OutputPath can discard them. Catch.ResultPath merges `{Error,Cause}` into the failed state's raw input rather than a partially transformed result.

### 2.3 Context and intrinsics

Support `Execution.{Id,Input,Name,RoleArn,StartTime}`, `StateMachine.{Id,Name}`, and `State.{Name,EnteredTime,RetryCount}`. Map ItemSelector may read `Map.Item.{Index,Value,Source}` with Source=STATE_DATA; inner iteration states receive such data through ItemSelector rather than implicit outer context. Only callback tasks expose Task.Token. Timestamps use RFC3339 UTC and fixture-defined precision. Context is read-only; internal actor IDs cannot masquerade as AWS ARNs.

Intrinsic allowlist: States.Format, StringToJson, JsonToString, Array, ArrayLength, ArrayGetItem, ArrayContains, ArrayPartition, StringSplit, MathAdd, UUID and JsonMerge. JsonMerge supports only `States.JsonMerge(a,b,false)` shallow merging. Reject all others at creation. Nesting is at most ten; each evaluation permits at most 10,000 AST operations and remains within the output budget. P0 fixtures pin escaping, Format, StringSplit delimiter behavior, MathAdd rounding/ranges, shallow merge and UUID format. Do not substitute Rust formatter/split semantics. Production UUIDs use secure randomness; deterministic tests inject a typed ID source rather than seed production randomness.

### 2.4 Choice and Wait

Choice supports short-circuit And/Or/Not, IsPresent/IsNull/IsString/IsNumeric/IsBoolean/IsTimestamp; String/Numeric/Timestamp Equals/LessThan/GreaterThan/LessThanEquals/GreaterThanEquals and their Path variants; BooleanEquals/Path and StringMatches. Do not coerce strings to numbers. IsPresent safely tests missing data; other missing-reference errors are runtime failures. Present values of inappropriate comparison types yield false. Distinguish missing, null and wrong type in fixtures. StringMatches implements AWS wildcard/escaping grammar, not user regex. Reject NaN/Infinity; preserve numeric comparison precision within the documented binary64 interoperability range.

Wait Seconds is nonnegative; timestamps must be valid RFC3339. Dynamic type errors fail. Past timestamps finish immediately. Apply AWS second-level timestamp truncation, compute remaining duration using wall time once, then use a monotonic timer to avoid NTP jumps. Overall deadlines still bound long waits. No dedicated sleeping thread per state.

## 3. Execution ownership and scheduling

One ExecutionActor owns execution data, all branch frames, attempt generations, retry counters, timers and history. Branches are frames, not an unbounded tree of Tokio tasks. A ready queue drives pure transitions, yielding after at most 64 transitions per tick. Bounded supervised JoinSet futures perform external calls and return typed results; pure evaluation never performs network IO.

Each frame holds state ID, entry generation, raw input, parent and phase: Entering, Calling, WaitingTimer, WaitingChildren, WaitingCallback or Completing. Timer entries carry generations. Reject stale timers/results before applying them. Completion, timeout, Stop and callback messages settle through the same owner exactly once. Results observed at or after the monotonic deadline cannot win a select race and become success.

```text
┌─ Supervisor ───────────────────────────────────────────────────────┐
│ open-execution permits │ JoinSet │ panic monitor │ cancellation    │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ ExecutionActor: one mutable owner ────────────────────────────────┐
│ ready frames → evaluate ≤64 transitions → history + read views    │
│      ▲                  │                       │                  │
│ timer heap         Task dispatch                ├─ terminal settle│
│      └─ Wait/Retry       ▼ bounded external calls└─ cancellation    │
│                         IntegrationPort                           │
│ frames ◀────────── result(attempt,generation)                      │
└───────────────────────────────────────────────────────────────────┘
```

Pure loops are bounded by transition/history/total-duration limits and cannot starve Stop or health. Registry mailbox defaults to 256; per-execution control/completion lanes to 64 each, with an independent stop/watch lane. Acquire downstream admission before dispatch. Waiting for Lambda capacity must not hold a scheduler worker or downstream dispatch slot indefinitely.

## 4. Retry, Catch and deadlines

Retry applies to Task/Parallel/Map. Scan retriers in order and use the first matching one. Maintain retry counts per retrier within a state entry. MaxAttempts counts retries after the first failure; zero disables retries. Defaults: IntervalSeconds=1, MaxAttempts=3, BackoffRate=2.0, JitterStrategy=NONE. Retry k waits `min(IntervalSeconds × BackoffRate^(k-1), MaxDelaySeconds)`; FULL samples uniformly between zero and that delay. Use checked/saturating arithmetic and the remaining overall deadline.

Validate IntervalSeconds in 1..99,999,999, MaxAttempts in 0..99,999,999, finite BackoffRate≥1, MaxDelaySeconds in 1..31,622,400, and NONE/FULL jitter. Effective work remains bounded by local transition/history budgets. States.ALL is the sole entry of the last matcher. States.TaskFailed matches task failures except States.Timeout, subject to uncatchable-error rules.

Only after retries exhaust does Catch scan in order. Without a match, propagate to the parent frame or terminate root execution. Retry starts from the original state input and reevaluates Parameters with updated RetryCount; it does not consume the failed output. Generate fresh attempt, Lambda request ID and callback token. External side effects may repeat. See [AWS error handling](https://docs.aws.amazon.com/step-functions/latest/dg/concepts-error-handling.html).

| Condition | Behavior |
|---|---|
| Task TimeoutSeconds/Path | Cancel resource, raise States.Timeout; Task Retry/Catch may handle it; reset per attempt |
| Machine TimeoutSeconds, Standard one-year ceiling or operator deadline | Execution TIMED_OUT; internal Catch cannot resume it |
| Callback heartbeat expiry | States.HeartbeatTimeout; starting point defined by callback integration |
| Lambda's own timeout | Preserve Sandbox.Timedout or the runtime-specific Lambda error |
| States.Runtime: unevaluable paths/types/expressions | Fatal, no Retry/Catch |
| States.DataLimitExceeded | Not matched by States.ALL; explicit matching may Retry/Catch it |
| Stop or parent cancellation | Abort/cancel; bypass user Retry/Catch |
| Actor panic or invariant failure | Supervised Rustack.InternalError, no automatic business replay |

Explicit DataLimitExceeded handling follows current AWS documentation rather than older claims that it is always uncatchable. Keep failure data bounded; repeated oversized transformations fail again. Runtime diagnostics retain Path/Intrinsic/ResultPath categories without embedding inputs. P0 pins precise boundary errors and history details for the profile.

## 5. Parallel and Inline Map

Parallel starts each branch with the same effective input and orders results by branch definition. Map selects an array via ItemsPath, applies ItemSelector and orders outputs by item index, never completion order. Empty Map returns `[]`. Normalize Iterator/ItemProcessor and legacy Parameters/ItemSelector once in the compiler.

Map MaxConcurrency=0 or omission selects the Inline limit of 40, further constrained by local task/frame budgets. Lower operational concurrency changes scheduling but not result order. MaxConcurrencyPath must resolve to a nonnegative integer. Static values above 40 are rejected by the local profile. Bound items to 10,000 and input to 256 KiB; active frames to 4,096 globally and 512 per execution. Create iterations lazily, not one spawn per item. Nested Map/Parallel share all budgets.

An uncaught branch/iteration failure stops new work, cancels and joins in-flight siblings, then settles the enclosing state. External side effects are not rolled back. Parallel/Map Retry repeats all branches/items, not just failed ones. Invalidate old generations/tokens before the next round; do not reset the execution deadline. All branch events belong to the root execution's history sequence with correct causal links.

## 6. Lifecycle and Rust engineering contract

Start transfers ownership to the supervisor, independent of its HTTP request. Stop marks cancellation, invalidates attempts, cleans up resources and commits the terminal state. Quarantined external resources remain supervisor-owned and visible in diagnostics. Restarting a supervisor restores its ability to accept work; it never silently replays an uncertain invocation or restarts a failed execution.

Apply AGENTS.md: thiserror/source errors; bounded actor messages and JoinSet; no Mutex-wrapped engine; non-exhaustive public DTOs and exhaustive internal state enums; forbidden unsafe/external panics and checked budgets; immediate serde validation; rstest/proptest/paused-time race tests; tracing skips payloads; Arc/Bytes and compiled IR avoid repeated parsing. Public compiler/runtime APIs provide examples and Errors sections. Configuration is centralized in the operations design, not embedded test shortcuts.

## 7. Cross-references

- ← [API/data model](./rustack-workflows-10-api-design.md)
- → [integrations](./rustack-workflows-40-integrations-design.md), [verification](./rustack-workflows-70-verification-plan.md)
- ↔ [operations](./rustack-workflows-60-operations-design.md), [decisions](./rustack-workflows-99-decisions.md)
