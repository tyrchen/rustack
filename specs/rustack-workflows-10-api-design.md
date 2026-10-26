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

Append history, update read views and settle terminal state atomically within the owning actor. Once DescribeExecution observes termination, history contains its terminal event. API input/output remain JSON strings. includeExecutionData=false omits execution data consistently without changing event linkage.

Use random opaque pagination handles backed by a bounded server-side registry. Bind scope, operation, filters, ordering, includeExecutionData and a read high-water mark. TTL is 24h. Stable execution sorting uses start time and ARN; history reversal uses event ID. Newly appended records do not change the cursor's captured boundary. History maxResults defaults to 100 and caps at 1000; 0 selects the default. Other list operations follow their Smithy limits. Expired, modified, cross-scope or argument-mismatched tokens return InvalidToken. Reject capacity exhaustion rather than evict live tokens.

Keep at most 25,000 events, also constrained by per-execution/global byte budgets. Reserve one event slot and byte allowance for termination. Imminent exhaustion fails the execution with local `States.HistoryLimitExceeded`; never truncate history and still report success. This locally identified error is advertised as a limitation. Clean up expired terminal histories only; preserve independent name tombstones.

## 7. Rust engineering contract

AGENTS.md § Error Handling: thiserror StepFunctionsError with source errors, mapped to protocol code/status without credentials, Docker internals or host paths. § Async & Concurrency: actor-owned registry/history, bounded mpsc/oneshot and supervised JoinSet; no Mutex-wrapped engine. § Type Design/Safety/Serialization: validated newtypes, checked arithmetic, forbid unsafe, immediate schema validation. § Testing: rstest error matrices and proptest ARN/idempotency/event-chain invariants. § Logging: typed IDs and redacted Debug, never input/definition/token. § Performance: Arc revisions and Bytes for original data. § Documentation: public API examples, Errors sections and ownership documentation.

## 8. Cross-references

- ← [PRD](./rustack-workflows-00-prd.md)
- → [ASL](./rustack-workflows-30-asl-design.md), [integrations](./rustack-workflows-40-integrations-design.md)
- ↔ [operations](./rustack-workflows-60-operations-design.md), [verification](./rustack-workflows-70-verification-plan.md), [decisions](./rustack-workflows-99-decisions.md)
