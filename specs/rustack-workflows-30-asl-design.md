# Workflows 30: ASL Compiler and State Machine Engine

Status: implementation contract · Depends on: [API/data model](./rustack-workflows-10-api-design.md)

## 1. Compile before execution

Create, Update and Validate share AslCompiler and produce immutable CompiledMachine objects. Never repeatedly parse definitions or resolve string Next targets during execution. Reject duplicate JSON keys and enforce byte/depth limits before constructing serde_json::Value. Business payloads are dynamic JSON; state definitions must become typed tagged enums rather than arbitrary maps interpreted throughout the engine.

Pipeline: bounded bytes → strict AST → field/state validation → scoped graph validation → Path/Intrinsic AST → integration resolution → indexed IR. StateId is a checked internal index. Next/Catch/Choice targets stay within their machine scope: no branch jumps to a parent or sibling machine. Loops are valid; do not require a DAG. Require a reachable terminal state; unreachable states are warnings.

Reject unsupported fields, JSONata, Assign, Credentials, dynamic Resource, Distributed Map and unavailable integrations during creation. Diagnostics carry JSON Pointer location, severity, stable Rustack code and bounded message without copying definition/input into logs. QueryLanguage may be omitted or explicitly JSONPath.

### 1.1 Executable sequential compiler and indexed IR

AslCompiler is synchronous, side-effect-free and configured with a validated TenantScope and trusted TaskSupport (Disabled/Lambda). The future composition root derives support from its installed adapter, not a request field; this compiler flag alone never advertises service readiness. Default construction supports pure workflows in the existing local default scope. Preserve original definition bytes on successful compilation for revision/idempotency use. Preflight at most1MiB,64 JSON container levels,65,536 members,1MiB strings and262,144-byte keys, including decoded duplicate-key rejection, before allocating the generic boundary tree. That tree is used only during compilation; execution consumes typed IR, never state maps or string targets.

Require an object with StartAt and nonempty States (at most1,000). Accept only optional Comment, Version="1.0", QueryLanguage="JSONPath" and positive integer TimeoutSeconds≤31,536,000. Validate state names/targets as1–128 UTF-8 bytes without controls. Assign StateIds in lexical state-name order and resolve every Next, Choice/Default and Catch target through the same scope's index. Allow loops, require at least one terminal reachable from StartAt, and warn for each unreachable state. Fail is terminal; a Task/Pass/Wait End=true is terminal. Potential runtime errors do not substitute for a reachable structural terminal. Bound compiled state/component/policy nodes to65,536; component-local bounds remain independently enforced.

The M1 compiler emits Pass, Task, Choice, Wait, Succeed and Fail only. Known Parallel/Map and child/callback integrations return a profile-unavailable diagnostic until their milestones; unknown types/actions return unsupported/invalid diagnostics. Enforce the field table in§2 before compiling shared data flow. Reject End=false, simultaneous/missing Next/End, top-level Choice Next/End, empty/more-than128 Choices, and every unknown state field. State-level QueryLanguage may be omitted or JSONPath; validate/drop Comment metadata. Pass Result is immutable dynamic JSON retained for bounded materialization at execution; it is exclusive with Parameters. Choice branches compile predicates without their Next field. Wait compiles only its one source. Succeed compiles only input/output paths. Fail preserves omitted/literal/dynamic error/cause fields without inventing an omitted error name; Error/Cause limits are256/32,768 bytes, literal/path pairs are exclusive, and dynamic paths must be reference paths or allowlisted intrinsic expressions yielding strings at execution.

Task Resource is a static complete ARN. Validate exact direct Lambda scope/name/optional qualifier or the partition-matching optimized lambda:invoke ARN; reject scope mismatch, malformed/trailing components and suffix-only .sync matches. A syntactically valid Lambda resource without TaskSupport returns dependency-disabled. Validate known static optimized Parameters fields FunctionName/Payload/Qualifier/InvocationType; FunctionName supports a name or same-scope ARN, InvocationType is RequestResponse and conflicting known qualifiers reject. Dynamic values remain checked by the future adapter. Task timeout is omitted, positive integer1..99,999,999 seconds or a singular TimeoutSecondsPath, never both. M1 rejects heartbeat fields because callback support is unavailable.

Compile Retry/Catch into immutable ordered policies (at most128 entries each and128 nonempty bounded256-byte/control-free error names per matcher). States.ALL must be alone and last in its policy; duplicate names in a matcher reject. Retry defaults/ranges follow§4, with zero MaxAttempts retained. Catch requires a same-scope Next and compiles absent ResultPath as replacement, null as discard, or a data-root reference. Recovery matching never handles States.Runtime; States.ALL/States.TaskFailed exclude DataLimitExceeded unless named explicitly, and TaskFailed excludes Timeout. These are the selected local matching rules. Timing is retained as typed metadata; actor retry counts/timers and jitter sampling remain separate work.

Successful Compilation contains Arc<CompiledMachine> and bounded warnings. The machine exposes immutable checked StateId/read views, original bytes and optional timeout; the IR stores compiled data flow, predicates, waits, resources, recovery and failure expressions. Invalid IDs fail explicitly. CompileError owns one Diagnostic and an optional typed JSON/ASL source; all Debug/Display redact definitions, literals, names and paths. Diagnostic codes/messages are static bounded Rustack-prefixed values, severity is ERROR/WARNING, and location is an RFC6901 pointer (empty for root; escaped ~ and /; at most4,096 bytes). A component failure may identify its enclosing component field rather than an exact expression token. Compilation stops at the first error; Validate later projects the same error or filtered/truncated warnings, without claiming AWS diagnostic ordering/count parity. CPU compilation must be run under bounded admission off the HTTP/registry scheduling loop when integrated.

Literal payload caching uses a64-byte minimum bounded-writer reservation rather than4KiB per scalar; otherwise a valid65,536-node artifact could retain hundreds of MiB in tiny literal buffers. This changes allocation behavior only, retaining all encoded byte/depth/member limits. Exact aggregate-budget fixtures must compile at65,536 nodes and reject65,537 while remaining below source/state limits.

```text
┌─ original definition bytes ──────────────────────────────────────┐
│ bounded JSON/duplicates → strict field/type validation            │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
┌─ compiler-owned scope index ──────────────────────────────────────┐
│ lexical names → StateId; resolve Next/Choice/Catch in this scope  │
│ data flow / predicates / waits / task and recovery metadata       │
│ reachability + terminal check; bounded warnings/compiled units    │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼
┌─ immutable compilation artifact ──────────────────────────────────┐
│ Arc<CompiledMachine> + original bytes + typed read views          │
│ diagnostics contain bounded pointers, no source-value excerpts   │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼ future registry/revision + actor
```

Tests require all six IR forms, graph loops/invalid targets/unreachable terminals, field/profile/dependency/scope rejection, static optimized metadata, timeout and Retry/Catch defaults/ranges, fail expressions, original bytes, pointer escaping/redaction, duplicate/deep/large definitions, exact state/choice/compiled-node limits and malformed-input properties. This compiler is not endpoint, actor, resource execution or complete M1 acceptance.

### 1.2 Structured scope compilation and staged runtime admission

P3 extends the immutable IR with Parallel and Inline Map instructions. Each branch or processor owns an `Arc<CompiledMachine>` with a separate lexical StateId index. All scopes share one immutable copy of the original root definition bytes and the same validated tenant scope. `original_bytes()` on any nested artifact returns that root source, not a reserialized branch document. `state_count()` remains scope-local. Retain unforgeable root-versus-nested provenance in each artifact: registry Create/Update reject a nested artifact before accepting a command, even when its instructions are sequential and its tenant matches. A branch cannot be promoted into a root revision whose fingerprint/Describe source still denotes the containing definition. No nested scope can resolve transitions against its parent or siblings; repeated state names in independent scopes are valid. Run graph validation for every scope, including unreachable branches, and aggregate warnings with full root-relative JSON Pointers.

Compilation traverses the already preflighted boundary tree once. A shared compilation context charges at most1,000 state definitions and65,536 state/component/policy units across the entire tree; neither counter resets at a nested scope. Root depth is0; at most16 nested branch/processor levels are allowed, independently of the64 JSON-container preflight limit. A Parallel has1–1,000 branches, additionally constrained by the shared state budget. Every scope has a nonempty States object and a reachable terminal. Branch/processor machines allow StartAt, States, Comment and QueryLanguage=JSONPath; Version and TimeoutSeconds are root-only. ItemProcessor additionally permits ProcessorConfig, an object with only optional Mode=INLINE; absent configuration, `{}` and INLINE are equivalent. Iterator has no ProcessorConfig. Distributed configuration and execution-type overrides fail compilation.

A normalized Map instruction stores input/output/result data flow, a compiled ItemsPath (default `$`), optional compiled item-selector template, a processor artifact, concurrency metadata, ordered recovery and continuation. Legacy Parameters becomes the item selector, **not** the Map data-flow Parameters step. Reject Parameters+ItemSelector and Iterator+ItemProcessor independently; current and legacy spellings from different pairs may coexist. Require exactly one processor spelling. Preserve JSON null versus omission: a present selector must be an object. ItemsPath and MaxConcurrencyPath must be singular reference paths; wildcard/slice sources fail compilation. Concurrency is a typed positive static limit or a compiled dynamic reference. Omitted/zero static limits normalize to40; static1–40 are retained; negatives, fractions, values above40 and simultaneous literal/path fields fail. Dynamic value/type/range checks happen on entry, against the effective Map input after InputPath. Count the item-selector template toward the shared node budget. Parallel uses the ordinary Parameters and result pipeline. Both instructions compile Retry/Catch in the enclosing scope, and expose Catch edges to enclosing graph validation.

The compiler's trusted `StateSupport` defaults to Sequential; explicit Structured enables construction/inspection of these artifacts for the frame executor. This flag is never a wire field or a readiness claim. Until the structured frame owner is installed, the standard ExecutionDriver constructor rejects structured artifacts and ExecutionSupervisor rejects structured drivers before accepting an execution; the real API retains Sequential support. An explicit pure frame-effect constructor may expose the structured protocol below without enabling supervisor/API admission. This is an internal rollout boundary, not M2 acceptance. Remove the runtime restriction only together with bounded frame scheduling, causal history, sibling cancellation and child integration verification.

```text
┌─ strict root JSON + shared original Bytes ───────────────────────┐
│ one budget: ≤1,000 states / ≤65,536 units / ≤16 nested scopes    │
└────────────────────────────┬────────────────────────────────────┘
                             ▼
┌─ enclosing machine: lexical StateIds ───────────────────────────┐
│ Parallel: branch Arcs ─┬─ independent machine/index → output[0] │
│                       └─ independent machine/index → output[1] │
│ Inline Map: one processor Arc; compiled selector + concurrency │
│ Next / Catch resolve only in this enclosing machine            │
└────────────────────────────┬────────────────────────────────────┘
                             ▼
          frame owner (next P3 dependency; API stays gated)
```

Compiler acceptance covers lexical reuse and cross-scope rejection for StartAt/Next/Choice/Catch, nested terminal validation and warnings, shared source ownership, current/legacy Map equivalence and conflicting pairs, exact state/unit/depth boundaries, nested Lambda dependency detection, pointer escaping/redaction and default API/driver refusal. P3 runtime acceptance remains the full scheduler/cleanup/nested-integration exit criteria in the implementation plan.

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

#### 2.2.1 Executable templates and data-flow phases

Template compiles a JSON object through the shared1MiB/depth64/member4,096/262,144-byte string/key source boundary. Existing Value inputs are borrowed and validated before compilation. Recursively compile nested arrays/objects into a private AST; dynamic fields ending in `.$` must hold an Expression string. Strip exactly one suffix and reject collisions after normalization, including nested objects. Literal field values are never inferred to be expressions from their text. Preserve array order and emit object fields in lexical normalized-key order. Limit nesting to32 template containers and4,096 total template/expression nodes, charging embedded expression nodes as well as their wrapper. Nonobject template roots, unsupported expressions and limits fail creation with redacted diagnostics.

Evaluation first prepares bounded borrowed literal views and owned expression results, sharing one intrinsic work/text/materialization budget across all dynamic fields. Stream the prepared view through the bounded JSON writer before constructing an owned Payload; never construct an oversized expanded Value tree and only then measure it. Preserve input/context on every failure. Template::evaluate uses the same explicit/default UUID sourcing rules as Expression. Template tests must cover collisions, literal/dynamic distinctions, arrays/context/intrinsics, aggregate expression limits and output expansion; callback history provenance/redaction remains the M3 integration's responsibility and is not advertised by this M1 component.

DataFlow compiles only InputPath, Parameters, ResultSelector, ResultPath and OutputPath from a component object; the containing state compiler owns the allowed field subset for each state type. Parameters/ResultSelector, when present, must be Template objects; explicit null is invalid. Absent input/output paths compile to `$`; null paths produce `{}`. Absent ResultPath replaces with result; null preserves the original raw state input; other ResultPaths use the data-root-only merge contract. Reject unknown fields and malformed paths without defaults after an error.

`prepare(original,context)` applies InputPath then Parameters and returns the resource's effective input. `finish(original,rawResult,context)` applies ResultSelector to the raw result, merges into the original state input and applies OutputPath. The execution owner must retain original separately across the external call; neither phase mutates it. Each phase shares one bounded evaluation budget across its operations and templates. Every selected/intermediate/result payload is validated before the next operation may discard it. Identity/default paths use the same selection/materialization behavior as explicitly supplied `$`. Finish requires an already validated rawResult, so an oversized connector cannot bypass its boundary through a selector or discarded result. The component performs no resource call, scheduling or accepted-command ownership; those remain actor responsibilities.

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

### 3.1 Sequential transition driver

The execution owner uses a synchronous `ExecutionDriver` over one captured `Arc<CompiledMachine>`. It performs at most one state/attempt transition per `advance` call; the actor batches at most64 calls before yielding. The driver owns original state input, effective context, entry time, per-retrier counters, total retry count, generation and phase. It performs no IO, spawn, sleep, clock reads, registry mutation or implicit resource retries. This is the sequential frame implementation; adding it does not advertise the HTTP execution service before the supervisor, registry, history and real bridge are installed.

Each call receives a paired wall/monotonic `ClockSample`. Reject regressing monotonic samples as an internal terminal failure; equal samples are valid. Derive the absolute execution deadline once from the minimum of configured positive duration (at most365 days) and machine TimeoutSeconds. Check it before processing a completion, a ready transition or Stop; at/after the deadline execution timeout wins. A settled outcome is immutable. Wall changes only affect the timestamp of a new entry and newly resolved absolute Wait sources. Already parked timers retain their original monotonic target.

Phases are Ready, Calling, Waiting (Wait or Retry), and Terminal. Commands return Continue, Dispatch, Park or Settled plus an optional cancellation obligation. Dispatch contains a machine-local state ID, unique monotonically increasing AttemptId, evaluated parameters and absolute attempt deadline. It transfers no resource ownership: the supervisor must retain each accepted invocation until settlement, even after the driver returns a cancellation obligation or terminal outcome. Complete accepts only the current Calling ID; old/duplicate completions are inert. A Task deadline observed by advance/complete invalidates the ID, returns its cancellation obligation once, and enters normal Timeout recovery. Stop invalidates an active ID and bypasses all user recovery. A retry may dispatch only after the owner has processed the previous cancellation/cleanup obligation; the pure driver cannot certify cleanup.

On entry, build `$$.State.Name`, EnteredTime (UTC RFC3339 nanoseconds), and RetryCount over the immutable base context supplied by the registry. Require an object base context; the later registry owns its Execution/StateMachine schema and scope. RetryCount starts at0, increments for every actual scheduled retry and does not reset when another retrier matches. Retries preserve EnteredTime and raw input; entering another state resets counters and captures a new entry time. Evaluate Parameters afresh on each dispatch. UUID evaluation uses the explicit fallible source; async owners run potentially blocking entropy/evaluation under retained bounded worker admission.

Pass evaluates prepare, selects its optional static Result (otherwise prepared input), then finish against raw input. Choice evaluates predicates against prepared input in source order, applies OutputPath after selecting a target, and raises States.NoChoiceMatched without a matching rule/default. Wait resolves against prepared input, stores its finished output once, and parks until its monotonic target; a zero wait completes immediately. Succeed applies input/output transforms. Fail evaluates each present literal/expression field against raw input/context, requires strings within256/32,768 bytes, and preserves omitted fields. Task resolves TimeoutSecondsPath against raw state input (before Parameters), validates a positive integer≤99,999,999, prepares parameters, and parks for its external result. Omitted Task timeout is bounded by the execution/operator deadline under this local profile.

Recovery scans the first matching retrier. An exhausted/zero-count matching retrier proceeds to Catch rather than falling through to a later retrier. Counts belong to each retrier for the current state visit. Compute exponential delay without unchecked duration conversion; saturate at the minimum configured cap and remaining execution duration before monotonic checked addition. FULL uses rejection sampling from an injected uniform64-bit source over the inclusive nanosecond range; bound rejection work to128 samples and fail internally if the source cannot provide an acceptable draw. A timer reaching the execution deadline cannot resume. Catch merges the bounded Error/Cause object into original input according to its compiled ResultPath, then enters the target. A failure in result processing follows Task recovery; Runtime is fatal and DataLimit requires explicit matching. Keep the typed AslError diagnostic separate from the static public failure cause. Every entered state/retry dispatch consumes the configured transition budget (1..1,000,000); before scheduling a retry, verify another dispatch is possible. Exhaustion fails immediately with Rustack.TransitionLimitExceeded rather than parking an impossible retry. Encoded payload byte/string/key limit failures map to States.DataLimitExceeded; bounded JSON syntax/depth/member failures remain Runtime. Encoding/allocation/entropy/invariant failures are Rustack.InternalError and bypass business recovery.

The driver exposes state, input, retry count, current deadline and stable outcome for its owner. It does not persist AWS history or expose an SDK endpoint; the actor must journal the official events and atomically publish read views under the independent history budgets before claiming execution acceptance. Tests drive clocks and completions explicitly: all six states, raw/effective input, dynamic Fail fields, default/missing Choice, Wait wall jumps, per-entry retry state, capped/FULL backoff, first-match exhaustion, catch merges, exact Task/execution deadlines, stale/duplicate completions, stop/timeout single settlement, and transition-loop exhaustion.

```text
┌─ execution owner / retained invocation supervisor ─────────────────┐
│ paired clocks + entropy + validated completion/stop                │
│                    ▼                                              │
│  ExecutionDriver: raw input + scoped IR + generation + phase       │
│       Ready → Calling / Waiting → Ready → immutable Terminal      │
│                    ▼                                              │
│ Continue / Dispatch / Park / Settled + cancellation obligation     │
│                    ▼                                              │
│ owner: official history + timers + calls + cleanup + read views    │
└───────────────────────────────────────────────────────────────────┘
```

### 3.2 Supervised sequential runtime and history publication

`ExecutionSupervisor` is owned by the future registry actor and contains the execution JoinSet, admission slots, dispatch/evaluation semaphores and quarantine records. Construction validates1..4,096 open executions,1..1,024 dispatches and1..128 evaluation workers; defaults128/32/8. Start accepts a fresh, unused compiled driver, reserves admission plus initial/terminal history capacity, registers a JoinSet job and returns a handle. Start requires an active Tokio runtime; invalid context returns a typed error rather than panicking. Dropping the handle never cancels accepted work. A lost control producer requests Shutdown instead of spinning on a permanently ready closed watch. Handles use independent watch lanes for first Stop and immutable Arc read views; no engine Mutex or per-request polling task is introduced. The supervisor must be reaped by its registry owner. Close rejects starts and signals Rustack.Shutdown; drain uses one absolute deadline and retains the same jobs/slots on timeout. Restart constructs a new accepting supervisor after the old instance is clean; it never replays uncertain business calls. Dropping a nonempty public supervisor transfers its bounded state to an explicitly detached fallback drain. That fallback has its own Drop guard: cancellation before first poll, panic, runtime shutdown or absence of a runtime retains unresolved slots/jobs/dispatch leases as process-lifetime quarantine, never as successful retirement. Normal joined drain releases the graph.

One execution actor owns its driver, journal and current call. All CPU/entropy evaluation runs in a retained spawn_blocking JoinSet under the shared evaluation semaphore. Keep a cheap immutable pre-operation checkpoint (Arc payload/context/machine, bounded retry counters, shared typed diagnostic). Sample Task completion at the owning actor before queueing its pure result transformation; this observation must precede the Task deadline. After the worker returns, recheck Stop and the absolute execution deadline before accepting effects. Discard an unpublished proposal and settle from the checkpoint if control/overall timeout won during evaluation. A late prepared dispatch also cannot issue work. No late dispatch or terminal success can bypass this final check. A pending Stop overrides an unpublished proposal and prevents new calls. Synchronous admission returns use one paired observation sample for post-return control/deadline checking, history and completion. Ready processing checks control between operations and yields after at most64 operations. A CPU panic becomes Rustack.InternalError without replay; the outer supervisor handles actor panics, publishes failure using its retained terminal reserve and keeps the slot quarantined when resource settlement cannot be proven.

LambdaTaskPort uses synchronous bounded admission returning a concrete channel-backed TaskCall immediately. The adapter's retained manager owns every accepted start future before returning. Core never starts an async admission future that can lose its eventual handle. An invocation sends Started at most once and one Completed result. The bounded event lane has capacity4. Clean completion confirms settlement; uncertain completion transfers its communication receipt into a QuarantinedTask that must stay beside the actual owned resource until joined cleanup calls settle. Call Drop requests cancellation only. Closed lanes never imply clean settlement. Core dynamically injects this synchronous object-safe trait without async-trait; async handle waits are concrete methods. This refines the logical start/handle contract in the integration design without changing the AWS API.

A completed/invalidated call moves to a retained cleanup JoinSet, holding its dispatch permit until Clean. Explicit ASL recovery may proceed while earlier uncertain calls remain owned, sharing the same bounded dispatch budget; no hidden business retry is added. At terminal publication, cancel current/retired calls and continue joining cleanup independently of HTTP observers. The open-execution slot remains reserved until all owned work settles. Lost cleanup channels or actor failures remain quarantined with their admission/dispatch capacity; drain reports failure instead of publishing a successful consistency snapshot. A waiter timeout never aborts or replaces owned work. The adapter must retain actual handles and report cleanup truthfully; the core receipt alone cannot prove an external resource is gone.

History is generated from official model DTOs and published together with outcome, stop timestamp and cleanup state in one immutable watch value. ExecutionStarted exists before Start succeeds. Enter/Exit and direct Lambda/optimized Task schedule/start/result/timeout events use official names; retries stay within one state visit. Sequential causal predecessors use the last committed event ID. A missing/failed result selection follows the raw resource-success event; Stop/overall expiry never manufactures a successful state exit. Invalidated completions are omitted. M2 must extend causal frames rather than apply this linear predecessor rule to concurrent branches.

Use an append-only persistent history view: immutable pages of at most64 Arc events, shared previous pages, and a copied bounded tail. A view never clones the full event vector or business trees on every append. Read pages validate start/limit (limit1..1,000), traverse at most25,000 records, preserve ascending IDs and can strip execution data without changing causality. The local data-disabled projection removes input/output fields and omits the optional parameter-bearing optimized TaskScheduled details object as a whole; it never fabricates replacement parameters. This conservative metadata difference is not a cloud differential claim. API pagination/high-water/4MiB page encoding remains registry/HTTP work. Snapshots retain charged event Arcs, so global accounting is released only after the last view is dropped. Bound event count to2..25,000 and per-execution charged bytes to1..256MiB; global capacity8MiB..4GiB, at least the per-execution cap. Charge bounded serialized event bytes plus fixed DTO/Arc overhead, before installing an event; include a4KiB terminal data allowance plus charged fixed DTO/page overhead held above worker lifetime. Ordinary events cannot consume that event/byte reserve. Oversized/exhausted ordinary or successful terminal history settles with the small reserved States.HistoryLimitExceeded failure. This is a local quota diagnostic. Internal event construction/encoding failures become Rustack.InternalError. Terminal history and read-view outcome always agree.

Typed worker/evaluation sources remain in redacted internal read-view diagnostics; they are not copied into protocol errors. Debug of a history view reports bounded counters rather than traversing its full chain. Test-only blocking worker gates have a separate RAII release guard so failed assertions cannot strand the blocking pool.

Tests must exercise actual asynchronous pure executions and channel-backed calls; caller loss, immediate completion/start races, Retry/Catch, timers, stop during calls/CPU admission, exact final-deadline checks, CPU/actor panic, dispatch/open saturation, retained cleanup/quarantine, repeated drain timeout and recovery, history limits/global accounting, stable old views, execution-data omission and25,000-event drop. These core tests do not advertise main-service readiness or replace real Lambda bridge/SDK acceptance.

```text
┌─ registry-owned ExecutionSupervisor ─────────────────────────────────┐
│ JoinSet + admission slots + global history reserve + quarantine      │
│   ┌─ execution actor ─────────────────────────────────────────────┐  │
│   │ Stop watch → driver proposals → final clock/control check     │  │
│   │        bounded retained CPU workers                          │  │
│   │ journal → immutable history/outcome watch → API read handles  │  │
│   │ Dispatch → TaskPort → current call → retired cleanup JoinSet  │  │
│   └───────────────────────────────────────────┬───────────────────┘  │
│ terminal visible; slot/dispatch capacity held └→ Clean / quarantine  │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.3 Deterministic structured frame-effect protocol

The frame interpreter emits effects; it does not spawn children or own Lambda calls. `ExecutionDriver::for_structured_owner` explicitly enables typed Parallel/Map effects for an owner that implements this protocol. `new` remains sequential and the current supervisor rejects any structured artifact, including a driver produced by the explicit constructor. This prevents an IR-only rollout from accepting incomplete service executions. No definitions are re-parsed during execution.

On Parallel/Map entry, process InputPath and the normalized Parameters stage once, then prepare an immutable `FanoutPlan`. The plan captures the machine-local state, fresh round AttemptId, original absolute execution deadline, effective input, current state context and child base context. Parallel child descriptors share the effective input and preserve branch-definition indices. Map resolves ItemsPath to an array, bounds it to10,000 items and the256KiB payload contract, then resolves MaxConcurrencyPath against the effective input. The selected local dynamic range is integer0–40, with0 normalized to40; reject missing/null/wrong-type/fraction/negative/over-limit values as States.Runtime, including for empty arrays. Static limits are already normalized by compilation. The local Inline profile remains array-only, even if broader AWS language documentation adds object iteration.

Map child inputs are prepared lazily by index. Without a selector, pass that exact array element, including JSON null. With ItemSelector/legacy Parameters, evaluate against the effective Map input, adding Map.Item.Index, Value and Source=STATE_DATA only to the selector's context. The selector observes the enclosing Map state's State.Name/RetryCount. Its resulting payload becomes the processor's raw input. Child base context shares Execution/StateMachine metadata but removes Map; inner states cannot implicitly read Map.Item. All descriptors and plans redact Debug output. Generating a descriptor can acquire UUID entropy and must run under bounded CPU ownership. A retained plan captures its round context; retries produce fresh plans and re-evaluate selection with the updated RetryCount.

The owner honors the plan's maximum concurrency and shared frame budgets, admits children lazily, and records each descriptor's index independently of completion order. `FanoutPlan::collect` accepts exactly one indexed output per descriptor, rejects missing/duplicate/out-of-range indices, and encodes the ordered output array through the bounded writer before allocating a result tree. The owner must also bound cumulatively retained result bytes while outputs arrive; collect-time validation alone is not a retention budget. An empty Map joins `[]` immediately without allocating child frames, while still applying ResultSelector→ResultPath→OutputPath. Both instructions merge into the state's original raw input.

The driver parks in JoiningChildren for a nonempty plan. Task completion cannot settle a round, and round completion cannot settle a Lambda attempt. Round completions carry the captured AttemptId: stale/duplicate/cross-kind results are inert. A successful join must be an array with the captured cardinality; invalid owner output is Rustack.InternalError. The owner reports either a bounded branch failure or a typed selector/aggregation failure after stopping admission, cancelling and joining siblings. Branch failures use the same ordered Retry/Catch policy as Task. For an omitted branch Fail Error, match policies using States.TaskFailed while retaining omission if no policy handles it. States.Runtime bypasses policies; explicit DataLimitExceeded remains recoverable. Preserve diagnostic sources for typed failures. A retry starts the complete round again from original raw input with a fresh attempt and new item descriptors; it never resets the root deadline. At/after that deadline, timeout wins over round completion. Stop invalidates a pending round and returns its cancellation obligation; repeated Stop, stale completion and later ticks cannot resurrect it.

```text
┌─ one frame interpreter ──────────────────────────────────────────┐
│ Ready → prepare Parallel/Map → FanOut(plan, attempt, deadline)   │
│                         │                 ▼                     │
│                    JoiningChildren ← owner: lazy indexed frames │
│                         │                 │                     │
│  retry ← ordered recovery ← failure + sibling cleanup joined    │
│                         │                                       │
│  next/end ← raw-input result pipeline ← ordered bounded outputs │
│ Stop/deadline → invalidate round → owner retains cancellation    │
└─────────────────────────────────────────────────────────────────┘
```

This protocol is the interpreter dependency of P3 scheduling, not proof of concurrent actor/history or actual child-Lambda execution. The frame owner must share the root transition budget and exact deadline across descriptors, hold frame/resource ownership until cleanup is known, and commit history causally before publishing execution views. Child construction must install the captured absolute deadline exactly, including when less than one second remains; calling the root constructor again with a duration is insufficient. Preserve the terminal failure origin separately from retained historical diagnostics: a diagnostic may refer to an earlier handled error. Propagate an actual child interpreter/internal terminal error through typed Evaluation, never flatten it into a business Branch failure that could trigger States.ALL replay. Its admission gate is removed only with the complete bounded owner and P01–P03 lifecycle verification.

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
