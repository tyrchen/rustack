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

#### 2.1.1 Executable path and payload foundation

The first `rustack-stepfunctions-core` slice exposes only implemented pure semantics: immutable Payload, compiled Path/ReferencePath and bounded result merging. It does not supply a registry, handler or advertised service capability. Core depends on shared JSON validation, not HTTP, Lambda or an SDK. Reuse the workspace winnow1.0 parser (current1.0.4 [API](https://docs.rs/winnow/1.0.4/winnow/)); no new external dependency is required.

Payload constructors validate at most262,144 original UTF-8 JSON bytes, depth64 and65,536 aggregate members through the shared duplicate-key preflight. Preserve original bytes for API idempotency and expose only an immutable parsed value. Constructors accepting an existing Value borrow it; validate structure and finish bounded serialization before cloning, so rejection never recursively drops a caller-owned deeply nested tree. Serialization must stop at262,144 bytes rather than build an oversized buffer first. All diagnostic Debug/Display implementations redact values, paths and source text. Typed source errors remain available without public input excerpts.

EvaluationContext has an independent internal envelope:4MiB,68 container levels and131,584 aggregate members, with each decoded string/key at most262,144 bytes. This accommodates full-size/depth/member-count Execution.Input and Map.Item.Value plus fixed context metadata. Compact exponent lexemes can expand during numeric serialization:65,535 copies of1e9 fit262,141 source bytes but encode to about852KiB. The pinned serializer uses at most24 bytes per numeric value; a conservative allowance of both original256KiB sources plus24 extra bytes per131,584 members and256KiB metadata remains below4MiB. Test both exponent-heavy inputs together and mixed exponent/string payloads, rather than deriving context capacity from string fixtures alone. It is immutable, constructed from a borrowed validated tree before cloning, and redacts Debug. Engine-owned typed builders must later constrain the actual context schema and per-field metadata. Selecting from context still materializes through the ordinary payload's256KiB/depth64/member65,536 boundary; the context root itself need not fit that output boundary. Converting an existing Payload into EvaluationContext shares its immutable Arc and requires no serialization.

Paths are at most1,024 UTF-8 bytes and128 segments. Dot member shorthand starts with an ASCII letter/underscore or non-ASCII non-control scalar; subsequent characters may also be ASCII digits. Bracket members use single quotes and support only escaped single quote/backslash; reject control characters and unterminated/unknown escapes. Bracket indices are decimal usize values with checked conversion; slices allow omitted lower/upper bounds, defaulting to zero/array length, and never negative indices, steps or whitespace. The whole string must parse. A general path may use either root; ReferencePath rejects wildcard/slice segments, and a separate result-target conversion rejects the context root. This is the local subset of the [AWS path contract](https://docs.aws.amazon.com/step-functions/latest/dg/amazon-states-language-paths.html), not full JSONPath support.

Evaluation borrows immutable payload/context values and bounds work to10,000 visited candidates/selected members per call. A wildcard/slice makes the result a Selection even with zero or one match; failed singular traversal is Missing, distinct from Single(null). Preserve array traversal order; explicitly sort object wildcard keys independently of serde_json feature unification. No missing/type mismatch is converted to null. Materialization returns a bounded Payload and raises a typed missing/data-limit/work-limit error. Selection materialization streams references through the bounded writer before building its owned value.

Result merging accepts only the validated data-root ReferencePath. The root replaces the original payload. Nested object members may be created only as objects; writing through scalar/null or a missing member immediately followed by an array index fails. Existing arrays require an in-range index; never grow an array. Apply edits to an owned copy, preserving the original payload on every failure, and enforce the full output depth/member/byte limits after the edit. Discarding a result is a separate data-pipeline decision, not a special path spelling.

```text
┌─ bounded source text ─────────────┐   ┌─ bounded path text ────────────┐
│ JSON preflight → Payload        │   │ winnow → Path / ReferencePath  │
│ original bytes + immutable tree │   │ root + checked typed segments │
└────────────────┬────────────────┘   └─────────────────┬──────────────┘
                 └──────────────────┬──────────────────┘
                                    ▼
┌─ pure evaluator / result merge ────────────────────────────────────┐
│ borrowed candidates;10,000 work units; deterministic selections  │
│ Missing / Single / Selection → bounded writer → validated Payload │
│ typed redacted failures; original input never mutated             │
└───────────────────────────────────────────────────────────────────┘
```

Tests must exercise the actual evaluator against independent examples in the semantic fixture catalog, exact source/compiled/expansion limits, hostile escapes/overflow, context access, consecutive wildcards, null/missing/empty selection, scalar merges and non-growing arrays. Property tests cover arbitrary UTF-8 parsing and quoted-key round trips; this foundation does not satisfy complete compiler/engine acceptance.

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

#### 2.3.1 Executable expression contract

Expression compiles a complete Path or an allowlisted States call, without evaluation during creation. Use the existing winnow lexer with a private typed AST. Bound each expression to4,096 UTF-8 bytes,1,024 nodes, ten nested calls and128 arguments per call. Full consumption is mandatory; reject surrounding whitespace, unknown functions, wrong arity, trailing commas, malformed numbers and unsupported syntax. Internal ASCII whitespace may surround arguments/commas. Arguments may be single-quoted strings, JSON numbers/booleans/null, compiled Paths or nested calls; object/array literals are constructed through Array/StringToJson. Numeric lexemes are at most128 bytes and use existing correctly rounded JSON conversion. These are local bounds, not full ASL parser support.

Decode apostrophe/backslash/brace escapes exactly once. String literals support only `\'`, `\\`, `\{` and `\}` escapes; reject dangling/unknown escapes. Business control characters already decoded from the outer JSON string remain data and are safely JSON-encoded in results. Preserve a format literal's escape provenance until Format processes it, so an escaped brace is not accidentally reinterpreted as a placeholder. Literal non-format braces must be escaped. Format accepts only `{}` slots, checks slot/argument equality and accepts scalar substitutions (string without JSON quotes; number/bool/null compact JSON spelling); array/object substitutions fail under the formal ASL scalar profile. A runtime first argument may itself be a path or nested expression yielding a format string. No host formatter, regex or string interpolation evaluates user syntax.

| Call | Arity | Selected value behavior |
|---|---:|---|
| Format |1–128| Scalar replacement; brace/backslash escapes; dynamic format checked at evaluation |
| StringToJson |1| Bounded duplicate-key-safe JSON decoding of a string |
| JsonToString |1| Compact bounded JSON text, then ordinary JSON string encoding |
| Array |0–128| Preserve argument order |
| ArrayLength |1| Array count |
| ArrayGetItem |2| Nonnegative JSON integer index; no growth, coercion or missing→null |
| ArrayContains |2| Bounded structural JSON equality; numeric representation differences use supported finite comparison |
| ArrayPartition |2| Positive chunk size rounded as below; stable final short chunk |
| StringSplit |2| Each delimiter scalar separates; discard empty fragments; empty delimiter preserves nonempty input as one fragment |
| MathAdd |2| Round each finite argument to nearest integer, ties toward positive infinity; require int32 operands and checked int32 sum |
| UUID |0| Fallible OS entropy, RFC4122 variant/version4; no hidden deterministic production seed |
| JsonMerge |3| Objects only, false third value; right-side keys replace left, no recursive merge |

MathAdd/partition rounding is pinned to the historical AWS-validated math fixtures, including negative halves; implement floor plus a fractional comparison, avoiding `floor(x+0.5)` rounding at adjacent representable values. ArrayGetItem deliberately requires an integer JSON number rather than rounding floats. For mixed numeric structural equality, apply the Choice interoperability range so large integer/float values cannot silently alias; equal integer representations compare exactly. Empty-delimiter split and checked result overflow are explicit local choices. Primary language/function contracts: [ASL](https://states-language.net/spec.html), [AWS intrinsics](https://docs.aws.amazon.com/step-functions/latest/dg/intrinsic-functions.html); historical boundaries are recorded in the [semantic memo](../docs/research/spike-stepfunctions-semantic-oracle.md).

Every evaluation shares10,000 AST/data work units,4MiB of scanned text and4MiB of cumulatively materialized encoded payloads. These local budgets include nested calls, array equality/partition members and string delimiter construction (at most1,024 unique delimiter scalars). Reject exhaustion with WorkLimit; never return a partial result. Each intermediate result independently satisfies the256KiB/depth64/member65,536 payload contract, including connector/context selections before another function can discard them. Encode arrays/partitions/merges through borrowed views and the bounded writer before building an owned result; bound variadic argument retention by the shared materialization budget. Allocation overhead is separate from encoded byte accounting. Callers cannot mutate source payload/context.

Expression::evaluate uses a concrete OS-backed UUID source. A separate synchronous UuidSource port enables typed deterministic/failure fixtures; validate injected UUID version/variant before publishing. Use getrandom0.4.3 fill and uuid's existing random-byte builder, preserving typed entropy failures rather than using a panicking random convenience API. Both packages are already in the workspace graph; no unrelated dependency version upgrade is required. System entropy can block during early boot: the future async actor must inject pre-acquired IDs or evaluate through a bounded retained blocking worker, rather than make an entropy syscall inside its scheduling loop. Diagnostics and Debug remain redacted. Tests execute the12 independent intrinsic catalog examples and historical half/split edge inputs, nested/escaped/path/context grammar, exact limits, type/missing errors, overflow, output expansion, work exhaustion and UUID success/source failure. Expression acceptance does not qualify template/pipeline/compiler or workflow execution by itself.

```text
┌─ bounded expression text ─────────────────────────────────────────┐
│ winnow tokens → private call/path/scalar AST                      │
│ arity + escapes + static format checks; no source in diagnostics  │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼ shared evaluation budgets
┌─ pure evaluator ──────────────────────────────────────────────────┐
│ immutable data/context → bounded arguments → intrinsic operation │
│ UUID source → fallible entropy      borrowed output views        │
│                    ▼                                             │
│ bounded writer → validated Payload → next expression/template    │
└──────────────────────────────────────────────────────────────────┘
```

### 2.4 Choice and Wait

Choice supports short-circuit And/Or/Not, IsPresent/IsNull/IsString/IsNumeric/IsBoolean/IsTimestamp; String/Numeric/Timestamp Equals/LessThan/GreaterThan/LessThanEquals/GreaterThanEquals and their Path variants; BooleanEquals/Path and StringMatches. Do not coerce strings to numbers. IsPresent safely tests missing data; other missing-reference errors are runtime failures. Present values of inappropriate comparison types yield false. Distinguish missing, null and wrong type in fixtures. StringMatches implements AWS wildcard/escaping grammar, not user regex. Reject NaN/Infinity; preserve numeric comparison precision within the documented binary64 interoperability range.

Wait Seconds is nonnegative; timestamps must be valid RFC3339. Dynamic type errors fail. Past timestamps finish immediately. Apply AWS second-level timestamp truncation, compute remaining duration using wall time once, then use a monotonic timer to avoid NTP jumps. Overall deadlines still bound long waits. No dedicated sleeping thread per state.

#### 2.4.1 Executable Choice predicate contract

Compile a Predicate independently of its containing rule's Next target; the machine compiler owns the target. A predicate is either a sole And/Or array (1–128 children), a sole Not object, or exactly Variable plus one supported comparison/test. Reject unknown/duplicate operators and invalid literal types, paths and patterns before execution. Variable and comparison Path operands are ReferencePaths. The public constructor borrows a Value and validates bounded structure before any recursive compilation:1MiB encoded JSON, depth64,4,096 members,262,144-byte strings/keys. Limit the resulting predicate to1,024 nodes and32 nested predicate levels; these local limits must become compiler diagnostics.

And/Or evaluate left-to-right and short-circuit, including errors in unvisited operands. IsPresent alone safely observes a missing reference; every other test/comparison raises a typed missing-reference error. Resolve both comparison references before type comparison; present values of the wrong type yield false. IsNumeric tests the JSON number type. Numeric comparisons accept integers in[-2^53,2^53] and floating-point values strictly inside(-2^53,2^53). Reject boundary floats conservatively because an original out-of-range decimal/exponent lexeme can round to exactly2^53; retaining only its parsed f64 cannot distinguish it from an exact boundary float. Reject other out-of-range values rather than silently aliasing large integers. Enable the existing serde_json float_roundtrip feature centrally so equivalent interior decimal/exponent forms convert correctly to binary64; arbitrary_precision remains disabled. Compile-time numeric literals outside the range reject. Boolean/string comparison never coerces another type. Timestamp literals parse at compilation; dynamic invalid timestamps compare false. Validate RFC3339 offsets with chrono, then represent the UTC whole seconds and the complete decimal fraction separately; remove trailing fractional zeros and compare exact fractions beyond nanosecond precision. Reject leap-second representations and do not equate complete timestamp strings lexically. Debit original literal and dynamic timestamp byte lengths before parsing/comparison.

StringMatches compiles a bounded glob: at most4,096 UTF-8 pattern bytes and1,024 tokens, literal Unicode scalars, `*` for zero or more scalars and only `\\*`/`\\\\` escapes. Braces/question marks are ordinary characters; reject dangling/unknown escapes. Collapse consecutive stars. Evaluate with checked UTF-8 offsets, no user regex and no unbounded recursion. A trailing star can accept the remainder directly.

Each evaluation has one shared10,000-unit budget for predicate/glob steps and1MiB aggregate compared string/timestamp bytes. Debit before work; exhaustion is a typed WorkLimit failure, never false. This prevents compound comparisons and glob backtracking from monopolizing the execution actor. These are local safety limits, not an AWS parity claim. Keep compiled predicates and diagnostics redacted. Tests cover the supported literal/Path operator matrix, logical order, missing/null/wrong type, timestamp offsets, binary64 boundaries, Unicode glob escapes/work exhaustion and borrowed extreme-depth rejection. Do not claim complete Choice state/compiler/engine acceptance from this evaluator alone. Primary operation names/short-circuit examples: [AWS Choice JSONPath rules](https://docs.aws.amazon.com/step-functions/latest/dg/state-choice.html#choice-state-rules-jsonpath).

#### 2.4.2 Executable Wait source contract

Compile WaitSource from an object containing exactly one of Seconds, SecondsPath, Timestamp or TimestampPath; the containing machine compiler owns Type, transforms and transitions. Reuse bounded source preflight/borrowed-tree validation before compilation. Static and dynamic seconds must be nonnegative JSON integers; reject floating-point spellings (including1.0), strings and booleans. The general ASL integer field ceiling is99,999,999; this STANDARD-only profile additionally caps a single resolved wait to31,536,000 seconds (365 days). Reject an overlong static Seconds at compilation and an overlong dynamic/datetime wait at entry. Other overall execution/operator deadlines can end the execution sooner.

Reference sources compile to singular ReferencePath values. Missing lookup raises Missing; present invalid type/range/date raises a distinct WaitValue failure and never silently defaults. Absolute dates use RFC3339 with uppercase T and uppercase Z when no numeric offset is present; permit explicit numeric offsets, reject leap seconds, lowercase separators, date-only text and invalid calendar values. Validate dates once for static timestamps and on entry for path timestamps. Truncate the target to its whole second before subtraction, retaining the current wall clock's fractional precision. Past/equal targets resolve to zero; future targets resolve to a checked std::time::Duration within the STANDARD cap.

Resolution is pure and receives the caller's sampled UTC wall time; it performs no IO, sleep or clock reads. The execution actor must sample its wall/monotonic pair once, add the resolved relative duration to that monotonic instant with checked arithmetic and register a generation-tagged timer bounded by the independent execution deadline. Do not recompute an absolute wait from wall time while the same state entry is suspended, or respond to subsequent NTP jumps. A new state entry evaluates its source afresh. WaitSource owns no task or timer handle and does not qualify scheduler/cancellation acceptance by itself. Tests cover all four source forms, exact caps, invalid/double fields, duplicates, path missing/null/wrong type, UTC offsets, fractional target truncation, fractional current time and past targets. Primary contract: [AWS Wait state](https://docs.aws.amazon.com/step-functions/latest/dg/state-wait.html).

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
