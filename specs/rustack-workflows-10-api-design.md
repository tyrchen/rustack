# Workflows 10: Step Functions API, Data Model and History

Status: implementation contract · Depends on: [PRD](./rustack-workflows-00-prd.md)

## 1. Protocol and crate boundaries

Add `rustack-stepfunctions-model`, `rustack-stepfunctions-http` and `rustack-stepfunctions-core`. Keep the ASL compiler/interpreter in core's `asl/` and `engine/` modules rather than publishing another crate prematurely. HTTP owns codecs and protocol errors, not execution semantics. Generate model types with the existing Smithy generator.

| Item | Contract |
|---|---|
| Source | [Official SFN Smithy](https://raw.githubusercontent.com/aws/api-models-aws/main/models/sfn/service/2016-11-23/sfn-2016-11-23.json); record source commit and SHA-256 when imported |
| Generator configuration | `codegen/services/stepfunctions.toml`; namespace `com.amazonaws.sfn`, service ID `AWSStepFunctions` |
| Protocol | `awsJson1_0`; POST `/`; Content-Type `application/x-amz-json-1.0`; target `AWSStepFunctions.<Operation>` |
| JSON names | API camelCase; preserve ASL PascalCase using explicit serde renames, never global case conversion |
| Signing | SigV4 service `states`; reuse gateway identity, scope checks and strict/skip modes |
| Local service name | Feature, SERVICES and capabilities use `stepfunctions`; AWS signing and ARN service remain `states` |
| Routing | Register before the S3 catch-all; unknown AWSStepFunctions operations must not fall through to S3 |

Future automation uses Makefile targets. The current generator needs protocol-aware numeric epoch-second timestamp serde for awsJson1_0; ordinary chrono serde produces strings. Before service generation, prove required/optional timestamps, fractional seconds and negative epochs in generated round trips. Keep protocol-default behavior scoped correctly so restJson/query timestamp bindings do not change accidentally. Resolve any union support required by the selected operations in the generator; do not hand-edit generated source.

### 1.1 Executable timestamp/model generation contract

Generate the pinned37-operation model into rustack-stepfunctions-model without advertising those operations as implemented. Add opt-in strict_model output configuration for this service: forbid unsafe, emit non-exhaustive public structs, deny unknown struct fields, derive typed-builder for structs with more than five fields (defaults only on nonrequired members), redact every DTO Debug field, keep error Debug/Display static while preserving typed source(), document fields/variants, and emit no blanket lint suppressions. Existing services retain their current generation policy.

For awsJson1_0/1_1 timestamp members, resolve member timestampFormat before target-shape format before protocol default. Default/epoch-seconds uses a generated epoch_seconds module; explicit date-time retains chrono serde; unsupported explicit formats fail generation. Other protocols retain their existing binding behavior. Apply required and optional serde adapters (optional includes default for omission); do not change plain strings which resemble dates. The pinned SFN model has no timestamp-valued list/map members; unsupported timestamp collections must fail generation rather than emit chrono string defaults.

The JSON adapter uses serde_json raw_value to preserve decimal numeric lexemes without binary-float rounding or a workspace-wide arbitrary_precision feature. Accept JSON numbers including exponent notation, at most128 lexical bytes; reject strings, required null, booleans, collections, invalid numeric grammar and chrono-range overflow with static diagnostics. Normalize to millisecond precision by truncating toward zero, including negative fractional epochs. Compute checked decimal scaling with bounded work; never cast a negative fraction to unsigned nanoseconds. Serialize finite chrono timestamps as exact numeric decimal seconds at the same precision, reject leap-second representations, omit absent optional members and accept optional null. This JSON-specific wire adapter is not a generic CBOR timestamp contract. HTTP still enforces its independent request envelope limit before model parsing.

`make codegen-stepfunctions` generates all source including the codec from repository templates and formats it. `make test-stepfunctions-model` checks actual generated required/optional fields, positive/negative/fractional/exponent/range/omitted/null cases and regeneration consistency. Generator tests cover format precedence, protocol isolation, serde-disabled combinations and strict output without lint ignores. The developer CLI accepts bounded UTF-8 arguments, ordinary config/model files up to1/16MiB, and a60s generation deadline using Tokio1.54.1 with explicit rt/macros/fs/time/io-util features. Validate Smithy2.0, honor configured overlay preservation/module exports and reject unsupported output.dir/custom error formats rather than silently ignoring them. Check the standalone tool graph with make audit-codegen/deny-codegen. Primary contracts: [Smithy JSON1.0](https://smithy.io/2.0/aws/protocols/aws-json-1_0-protocol.html#shape-serialization), [timestamp precision/precedence](https://smithy.io/2.0/spec/protocol-traits.html#timestampformat-trait), [RawValue](https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html).

```text
┌─ AWS SDK clients ──────────────────────────────────────────────────┐
│ POST / + AWSStepFunctions.*     JSON strings(input/definition)      │
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼ identity / body / deadline boundary
┌─ stepfunctions-http ───────────────────────────────────────────────┐
│ Smithy codec → wire validation → typed commands → protocol errors  │
└─────────────────────────────┬──────────────────────────────────────┘
                         bounded mpsc
┌─ core / RegistryActor ──────▼───────────────────────────────────────┐
│ immutable machine revisions │ execution-name tombstones │ tokens  │
│ serialized Create/Update/Start/Stop; owned replies for read views   │
└──────────────┬───────────────────────────────┬──────────────────────┘
               ▼                               ▼
┌─ ExecutionActor ──────────────┐   ┌─ bounded history/read views ────┐
│ revision, frames, generations │──▶│ event sequence, terminal data   │
│ task handles and timers       │   │ snapshot DTOs, no live handles  │
└───────────────────────────────┘   └────────────────────────────────┘
```

### 1.2 Typed HTTP boundary before registry wiring

`rustack-stepfunctions-http` owns a concrete Hyper service, an object-safe async StepFunctionsHandler port, typed Request/Response enums for the15 M1 actions and three callback actions, immutable RequestScope and per-call RequestContext. Native async syntax with async-trait is justified by Arc<dyn StepFunctionsHandler>; no default/echo/unimplemented production handler is supplied. Model presence and callback transport types do not advertise core support. Register the service before S3 only when the real core adapter lands; keep feature/capability publication coupled to that registration.

Claim any exact AWSStepFunctions target family, including unknown actions, duplicate occurrences and a missing action suffix, independent of method/path; never forward those to S3. Require POST at root without query, one bounded ASCII target/content-type, JSON1.0 media type (optionally UTF-8 charset), no compression, and unambiguous Content-Length/Transfer-Encoding. Bound the whole header map to64 entries/32KiB, each value≤8KiB; target/content-type≤256 bytes. Unknown actions return UnknownOperationException/404 per the current [SFN common-error contract](https://docs.aws.amazon.com/step-functions/latest/apireference/CommonErrors.html); known actions outside the18-action transport profile return UnsupportedOperation/400. This updates the initial custom unknown-operation status in generator configuration. These checks are local rejection policy, not a measured full AWS error-precedence oracle.

Collect through the shared source-level BodyBudget capped at2MiB, with a configured total request deadline covering collection/authentication/dispatch/encoding. Reject every trailer frame before dispatch, including unknown and oversized trailer fields; trailers must not bypass the initial header limits. Authenticate the original bytes before treating a zero-byte body as `{}`. Explicit development mode skips verification; strict construction without a provider fails. Strict requests reuse ordinary SigV4 payload verification, require signing service states and the configured region, and bind x-amz-target in SignedHeaders. Scope partition/region/account comes only from validated operator configuration, never unsigned request fields. Authentication failures emit a static AccessDeniedException/403 while preserving typed sources in redacted diagnostics. This remains the gateway's local trust profile, not an IAM evaluator.

Preflight JSON (depth16,4,096 aggregate members,128-byte keys and1MiB maximum strings) before typed DTO construction. Strict generated DTOs reject unknown/duplicate fields. Validate operation-specific string/collection byte bounds from §4 before calling the handler; traceHeader accepts0–256 ASCII bytes, including empty text and spaces, rather than using the nonempty identifier grammar. Business JSON input/output text is independently bounded/validated, while definition strings remain compiler-owned so Validate can return FAIL diagnostics. Domain ARN/role/profile/ASL semantics remain core-owned and must validate before actor commands. Request/response/context/body/config Debug never expose definitions, input/output, task tokens, headers or credentials.

Handler response variants must match the request operation. Encode only the selected DTO, preserving numeric timestamps, with a bounded8MiB writer before returning200; failure/oversize/mismatch returns static InternalFailure/500 without replaying a mutation. History pagination later caps each encoded page at4MiB and may return fewer than maxResults with a consistent continuation token; individual events are never split/truncated. Success/error responses carry JSON1.0 content type, x-amzn-requestid and server headers; errors use short __type/message plus x-amzn-errortype. Handler messages are bounded to1KiB; invalid error status/message becomes InternalFailure. IO/JSON/auth sources are retained but never copied to public diagnostics. Final monotonic deadline checks prevent a late result becoming success. Dropping/timing out the HTTP waiter does not authorize rollback/cancellation of accepted core commands; the actor owns execution lifetime.

The embedding gateway owns bounded connection/request admission; this service spawns no per-request work. Tests cover all typed action mappings, empty/normal JSON, header ambiguity and nonfallthrough claims, body limits/slow progress, duplicate/unknown keys, operation-specific bounds, signature/service/region/target binding, static error envelopes, response correlation/byte limits, and deadline/abandoned caller behavior. The network fixture owns all connection tasks in a JoinSet, signals stop on shutdown, and retains its root JoinHandle after a cleanup-waiter timeout so the same join can resume. An ordinary SDK against a controlled typed handler can qualify this transport only; it cannot satisfy registry, ASL or Lambda-execution acceptance.

## 2. API coverage

| Milestone | Operations | Required behavior |
|---|---|---|
| M1 | CreateStateMachine, UpdateStateMachine, DeleteStateMachine, DescribeStateMachine, ListStateMachines | Immutable revisions, STANDARD only, deletion lifecycle |
| M1 | ValidateStateMachineDefinition | Same compiler as create/update; OK/FAIL, diagnostics, severity, maxResults, truncated |
| M1 | StartExecution, DescribeExecution, ListExecutions, StopExecution | JSON strings, idempotency, asynchronous execution, single terminal settlement |
| M1 | GetExecutionHistory, DescribeStateMachineForExecution | Read the revision captured by this execution, not latest |
| M1 | TagResource, UntagResource, ListTagsForResource | Atomic validation; basic IaC create/read/update/delete coverage |
| M3 | SendTaskSuccess, SendTaskFailure, SendTaskHeartbeat | Tokens bind to execution, scope and attempt generation |

Other known actions return HTTP400 local `UnsupportedOperation`, documented in capabilities; unknown actions return protocol UnknownOperationException. EXPRESS, qualified state-machine ARNs, publish/version/alias requests are explicitly rejected. StartSyncExecution is not a shortcut for awaiting STANDARD workflows.

Create/Update accept absent loggingConfiguration or level OFF. Enabled tracing, nondefault encryption and publish=true return ValidationException. roleArn is validated metadata, not IAM policy evaluation. Delete marks the machine DELETING, rejects new Start/Update, and lets accepted executions finish. When the last execution closes, remove mutable definition and tags; execution/history/name records retain their independent TTLs. Repeated deletion during DELETING is idempotent; after removal return StateMachineDoesNotExist.

CreateStateMachine is idempotent for the same name, raw definition and supported effective type/logging/tracing/encryption/publication settings. A repeated request with different roleArn/tags returns the existing resource without modifying those fields; a conflicting definition returns StateMachineAlreadyExists. Validate unsupported fields before considering a request idempotent. This follows [AWS CreateStateMachine](https://docs.aws.amazon.com/step-functions/latest/apireference/API_CreateStateMachine.html). Explicit Update changes the latest revision and affects only subsequently started executions.

StopExecution of RUNNING work requests cancellation and returns the committed stopDate after terminal settlement, bounded by the control deadline. A repeated Stop on an already terminal record returns its existing stopDate without rewriting outcome/history; an absent execution returns ExecutionDoesNotExist. Concurrent success/Stop settles once. If resource cleanup remains uncertain, retain quarantine ownership and expose failure diagnostics rather than rewriting a completed execution to RUNNING. Read/List operations validate every supported filter; version/alias/MapRun/redrive filters are rejected until implemented.

## 3. Domain types and ownership

Follow AGENTS.md § Type Design & API: private validated fields, TryFrom/FromStr, non-exhaustive public structs and redacted Debug. Multi-field construction uses typed-builder.

| Type | Fields and invariants | Owner |
|---|---|---|
| TenantScope | Validated partition, region, account | Gateway creates; core revalidates |
| StateMachineName / ExecutionName | 1–80 ASCII bytes, `[A-Za-z0-9_-]` | RegistryActor |
| StateName | 1–128 UTF-8 bytes; no controls; exact matching without normalization | Compiler |
| StateMachineArn / ExecutionArn | Structured `states` ARN, matching scope, ≤256 bytes | Boundary newtypes |
| DefinitionRevision | Revision ID, original definition, hash, Arc<CompiledMachine>, role/config | Immutable Arc |
| ExecutionRecord | ARN, captured revision, raw and parsed input, start/stop times, outcome | ExecutionActor |
| TaskAttemptId | Execution UUID, frame, entry generation, retry ordinal, task UUID | Engine |
| HistoryEvent | AWS event type/details, id, previousEventId, timestamp | Execution history owner |
| ResourceHandle | Lambda invocation, owned child or callback handle | Engine; must be settled |

ARN shapes are `arn:<partition>:states:<region>:<account>:stateMachine:<name>` and `...:execution:<machine>:<execution>`. No version/alias suffixes or MapRun labels initially. RegistryActor linearizes revision updates against execution starts.

Validate execution-role metadata as an IAM role ARN in the same partition/account, with empty region, a bounded role path/name and≤256 total bytes. Do not parse it with the state-machine ARN grammar or accept arbitrary service ARNs. This is structural validation, not an AssumeRole operation or permission evaluation.

Use `Running` versus `Terminal(Succeeded|Failed|TimedOut|Aborted)` domain states. Terminal variants carry either output or error/cause, preventing combinations such as RUNNING with stopDate. Map to AWS RUNNING/SUCCEEDED/FAILED/TIMED_OUT/ABORTED; do not expose unsupported PENDING_REDRIVE.

### 3.1 Executable registry ownership and resource lifecycle

Implement a reusable core `Registry` handle plus unique `RegistrySupervisor`, backed by one bounded actor. Accept already compiled immutable machines and validated Payload values: parsing/compilation belongs to the embedding API's retained bounded preparation owner, never a hidden unbounded task per registry command. Registry does not implement an HTTP trait or advertise endpoints. The later binary adapter maps the15 M1 requests and installs the real Lambda bridge; no controlled handler is a production fallback.

Provide private-field StateMachineName/ExecutionName (ASCII1..80), scoped StateMachineArn/ExecutionArn (whole string≤256, exact unqualified grammar), RoleArn and Tags. Parse ARNs with FromStr, build them from validated names/scope, and compare partition/region/account again at registry admission. Roles use IAM with an empty region and matching partition/account; nonempty slash-delimited visible-ASCII path components, final role name≤64 with IAM name characters and total≤256. Tags permit Unicode L/Z/N plus `_ . : / = + - @`, independent byte caps, at most50 and no case-insensitive aws: key prefix; reject duplicate keys in one mutation. Untag validates every key and the whole collection before mutation.

Default caps:256 current machines,1,024 retained revisions and independently1,024 retained tag versions,10,000 retained execution records,100,000 names,128 accepted commands,24h execution retention. Allow respectively1..4,096 /1..4,096 /1..100,000 /1..1,000,000 /1..1,024 and60s..90days. The revision/record counts are allocation ownership limits, not exact byte-memory claims; existing hard compiled/payload/history limits still apply. Revision, tag-version and execution record permits stay inside shared immutable allocations, so a retained read view remains charged after deletion/expiry. Reserve a replacement revision before releasing the old one; reject saturation atomically. No premature live-record/tombstone eviction. A full accepted-command pool rejects synchronously; queue admission transfers command ownership independently of any response observer. A timed-out reply waiter can resume the same receipt; completion or sender closure consumes its receiver before returning, so repeated reads fail with a typed error.

A DefinitionRevision retains a secure UUID, SHA-256 of exact source bytes, compiled Arc and validated role. Create is idempotent for the same raw source under the supported effective STANDARD/OFF profile, ignoring replacement role/tags without mutating them. Unsupported wire settings must be rejected by the future adapter before entering this profile. Explicit Update creates a new UUID even for equivalent source and supports definition-only/role-only updates; tags remain separate. Scope and all fields validate before command enqueue and are rechecked at the actor boundary. Missing/deleting resources are typed distinct failures.

Start receives an optional validated execution name, bounded parsed input and optional bounded ASCII trace header; omitted names use a secure UUID. The actor samples start clocks, captures latest revision, compares raw input for a still-running name, reserves record/name/runtime/history admission, constructs complete Execution/StateMachine context and registers the fresh driver without awaiting between claim and registration. On rejection no name/record is published; on success caller loss does not cancel it. A name for a closed execution always conflicts. Claims are keyed by execution ARN, so deleting/recreating a same-name machine cannot reuse that ARN during its tombstone. Captured resource UUID distinguishes old and recreated machine lifecycles.

Expose immutable MachineSnapshot and ExecutionSnapshot values, including captured revision, raw input, trace/start dates, atomic engine view and a cloneable execution observer/control handle. Describe-for-execution reads that captured revision after updates/deletion. List snapshots are bounded actor-owned high-water captures for the later pagination adapter; a snapshot never rewrites a previously returned terminal view. Stop validates optional error/cause before enqueueing an owned Stop command. The actor signals the engine once; its caller awaits terminal publication on the independent control/read lane outside the registry actor, preserving omitted or explicitly empty fields. Dropping the Stop reply observer does not discard an accepted cancellation. Concurrent Stop/success and repeated Stop use the original terminal outcome/date.

Maintenance reaps execution supervisor jobs, observes terminal publication, timestamps closure monotonically once, expires record retention only after confirmed cleanup and independently expires names no earlier than90days after observed closure. Pending/Quarantined cleanup retains its record/revision/name graph beyond those TTLs until physical settlement is proven. A1s timer plus command-boundary maintenance drives deletion: DELETING rejects Start/Update, allows existing executions, and removes mutable metadata/tags after its last captured resource execution becomes terminal. Cleanup may remain quarantined independently of business termination. Repeated Delete while DELETING succeeds; after removal it is absent. ListExecutions atomically requires a currently present machine (ACTIVE or DELETING), then returns retained records matching its ARN, including earlier incarnations. DescribeExecution still reads retained records after machine removal. This local reincarnation rule is explicit; the missing-resource error follows [AWS ListExecutions](https://docs.aws.amazon.com/step-functions/latest/apireference/API_ListExecutions.html). All list ordering is deterministic (machines by creation time/ARN; executions newest start/ARN first).

Secure UUID entropy is acquired by at most one retained blocking preparation job, with the accepted envelope/permit held above that job in the root state; cancellation cannot manufacture worker completion. Definition hashes are prepared by the embedding caller before admission. Maintenance builds one set of running resource identities, avoiding a machines×records scan.

Closing the unique supervisor rejects new commands, drains already accepted commands, closes the execution owner and joins the same work through observer cancellation/expiry. No HTTP/control timeout restarts an execution or grants rollback. The root owns mutable state through a Drop guard before spawn; abnormal cancellation/panic/runtime loss preserves the bounded unresolved graph as process-lifetime quarantine. Permanent execution quarantine fails shutdown and retains revision/record/name ownership. A clean joined shutdown releases registry allocations except caller-retained snapshots. Root Drop also synchronously closes the execution owner before retaining its unresolved graph. Debug/Display never expose role, names, ARN, tags, definition, input, tokens or host sources; typed sources remain internal.

```text
┌─ preparation owner (future binary API adapter) ───────────────────┐
│ bounded compiler / Payload parse → validated domain commands      │
└─────────────────────────────┬────────────────────────────────────┘
                              ▼ bounded accepted commands + replies
┌─ RegistryActor + Drop guard ─────────────────────────────────────┐
│ machines → Arc revision + retained permit                         │
│ names (90d) ← Start transaction → records → captured revision     │
│ Delete/Update/Tag          │          │                            │
│ monotonic maintenance     ▼          ▼ immutable snapshots         │
│                   ExecutionSupervisor → real LambdaTaskPort       │
│ close → drain same runtime → Clean / retained quarantine          │
└──────────────────────────────────────────────────────────────────┘
```

Qualification must cover concurrent raw-input idempotency, update/captured-revision isolation, delete/recreate names, omitted/empty Stop fields, cancellation after accepted commands, registration-failure rollback, independent record/name expiry, capacity held through retained snapshots, Unicode/duplicate/reserved tags, scoped ARN/role rejection, final history/view agreement and resumable shutdown. HTTP pagination, full SDK/main routing and final image/platform acceptance remain separate required consumers.

## 4. Input validation and limits

| Boundary | Validation |
|---|---|
| Control request body | Local 2 MiB limit, independently enforced before deserialization; JSON escaping counts toward this envelope limit |
| Decoded definition | 1,048,576 UTF-8 bytes; duplicate keys rejected; JSON depth ≤64 |
| Execution input/output and each transformation | 262,144 UTF-8 bytes; omitted input becomes `{}`, explicit empty string is invalid, JSON null is valid |
| Input equality | Preserve decoded raw UTF-8 string; byte equality, without whitespace normalization |
| ARN / trace / pagination token | 256 / 256 ASCII / 2,048 bytes |
| Tags | ≤50; key 1–128 bytes, value 0–256 bytes; AWS tag character allowlist, reserved aws: prefix rejected |
| Error / cause | 256 / 32,768 bytes; reject oversized client input; bounded internal causes carry an explicit truncation marker |
| Compiled structure | ≤1,000 states total, machine nesting ≤16, ≤128 branches per Parallel, path ≤1,024 bytes, intrinsic expression ≤4,096 bytes |

The 2 MiB envelope is a local transport policy, not exact AWS request-size parity; it can reject heavily escaped or metadata-heavy definitions despite their decoded size fitting. AWS's independent definition/payload quotas still apply. Payload strings allow legal JSON text; identifier ASCII restrictions must not corrupt business data. Every collection has count and aggregate-byte limits. Unknown API/config/ASL fields are rejected, not collected without bounds. Smaller local structural limits must appear in validation diagnostics.

## 5. StartExecution idempotency and revision isolation

Atomically reserve `(scope, machine identity, execution name)` and capture a revision. Validate input first, then look up the name. A RUNNING execution with identical raw input returns its original ARN/startDate, even after a definition update. Different input or any closed execution returns ExecutionAlreadyExists. Name tombstones last at least 90 days after closure, independently of shorter history retention. If tombstone capacity is exhausted, reject new starts rather than reuse names early. See [AWS StartExecution](https://docs.aws.amazon.com/step-functions/latest/apireference/API_StartExecution.html).

Generate a secure random UUID when name is absent. Delete/recreate must not allow collision with an existing execution ARN; old tombstones continue blocking the corresponding ARN until expiry.

Ordering: validate/compile → reserve admission → atomically claim name and create RUNNING record plus ExecutionStarted → register execution actor → return200. Registration failure rolls back the reservation and permits. A lost HTTP response does not undo a started execution. Disconnecting after Start never cancels it. Each execution uses its captured machine revision; Lambda alias resolution happens per Task attempt, as specified in the integration design.

## 6. History and pagination

History IDs increase from 1. previousEventId records the causal predecessor, not merely the most recently appended event across concurrent branches. Required events include ExecutionStarted/Succeeded/Failed/TimedOut/Aborted, Entered/Exited for supported states, LambdaFunctionScheduled/Started/Succeeded/Failed/TimedOut for direct integrations, and TaskScheduled/Started/Succeeded/Failed/TimedOut for optimized integrations. M2 adds official Parallel/Map/iteration event types and TaskSubmitted after an optimized .sync child is accepted, before its terminal result. Retry creates multiple task attempts within one state entry, not a fabricated AWS RetryScheduled event; internal retry scheduling belongs in tracing.

The core implementation uses [ASL§3.2](./rustack-workflows-30-asl-design.md#32-supervised-sequential-runtime-and-history-publication): shared64-event immutable pages and charged retained views with terminal capacity reserved above worker lifetime. Its conservative data-disabled projection omits input/output and the optional optimized TaskScheduled details object containing required Parameters; this local metadata difference is documented rather than claimed as cloud differential evidence. Registry pagination and main-service registration remain separate acceptance gates.

Append history, update read views and settle terminal state atomically within the owning actor. Once DescribeExecution observes termination, history contains its terminal event. API input/output remain JSON strings. includeExecutionData=false omits execution data consistently without changing event linkage.

Use random opaque pagination handles backed by a bounded server-side registry. Bind scope, operation, filters, ordering, includeExecutionData and a read high-water mark. TTL is 24h. Stable execution sorting uses start time and ARN; history reversal uses event ID. Newly appended records do not change the cursor's captured boundary. History maxResults defaults to 100 and caps at 1000; 0 selects the default. Other list operations follow their Smithy limits. Expired, modified, cross-scope or argument-mismatched tokens return InvalidToken. Reject capacity exhaustion rather than evict live tokens.

Cap each complete encoded history page at4MiB, including metadata and escaped JSON string contents. Return fewer than maxResults when needed, with a continuation token at the next event; never split or truncate an event. This leaves headroom below the transport's8MiB response cap.

Keep at most 25,000 events, also constrained by per-execution/global byte budgets. Reserve one event slot and byte allowance for termination. Imminent exhaustion fails the execution with local `States.HistoryLimitExceeded`; never truncate history and still report success. This locally identified error is advertised as a limitation. Clean up expired terminal histories only; preserve independent name tombstones.

## 7. Rust engineering contract

AGENTS.md § Error Handling: thiserror StepFunctionsError with source errors, mapped to protocol code/status without credentials, Docker internals or host paths. § Async & Concurrency: actor-owned registry/history, bounded mpsc/oneshot and supervised JoinSet; no Mutex-wrapped engine. § Type Design/Safety/Serialization: validated newtypes, checked arithmetic, forbid unsafe, immediate schema validation. § Testing: rstest error matrices and proptest ARN/idempotency/event-chain invariants. § Logging: typed IDs and redacted Debug, never input/definition/token. § Performance: Arc revisions and Bytes for original data. § Documentation: public API examples, Errors sections and ownership documentation.

## 8. Cross-references

- ← [PRD](./rustack-workflows-00-prd.md)
- → [ASL](./rustack-workflows-30-asl-design.md), [integrations](./rustack-workflows-40-integrations-design.md)
- ↔ [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)
