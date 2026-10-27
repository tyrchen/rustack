# Rustack Specs 索引

本文件是 specs 的导航入口；文件中的 Draft / Implemented 状态以各原文为准，不代表经过本次运行验收。

## Step Functions and Lambda Runtime — new specification set

Status: P0 evidence complete; P1 remains in progress. Implemented and independently reviewed foundations include the shared runtime/HTTP/relay/client, bounded Docker wire, immutable image resolution and leased deployment/provider settlement, persistent directory ownership, allocation policy, retained environment lifecycle and FIFO/warm/LRU dispatch. Delivery-based handler clocks and invocation-scoped observed log windows are qualified through actual image execution. Per-call pool cleanup settlement is retained above dispatch workers and resumable after cancellation or waiter expiry. The Docker Executor/provider/HTTP composition is independently reviewed and exercised through ordinary boto3 against actual images, using controlled fixture readiness. Durable container intents now survive directory reacquisition; historical recovery retains unknown absence and retires only exactly verified identities, with independently reviewed, actual pair-retirement evidence on both retained daemons. Startup network qualification pins full bridge IDs; actual pinned Rust probes validate kernel resource/isolation settings and joined retirement without granting readiness. B01 dependency security, B03 strict boundaries, B04 deployment validation and B05 post-bootstrap initialization classification are closed. See the [implementation ledger](./rustack-workflows-90-impl-plan.md#11-implementation-execution-record) and [review](./rustack-workflows-93-review.md) for exact evidence.

The ordinary Docker image path now consumes production provenance/recovery, typed pull/configuration, immutable relay/network/kernel/optional endpoint receipts and retained provider/listener/shutdown ownership under D81. Real signed SDK and ASL image execution is separately qualified from the earlier controlled component fixtures. Full M0 and final image/platform release acceptance remain in progress. The P2 generated Step Functions model/numeric timestamp foundation is implemented and independently reviewed. The typed HTTP transport is implemented and independently reviewed, including signed ordinary SDK qualification against a controlled handler. Bounded compiled paths/payloads, Choice predicates, pure Wait sources, the12-call intrinsic expression profile, template/data-flow phases, the sequential machine compiler/scoped IR, deterministic six-state transition driver and supervised runtime/history/task-port owner are implemented and independently reviewed, including context headroom, exact timestamp fractions and conservative numeric comparisons. The binary Lambda bridge now invokes the real shared provider with owned admission and resumable cleanup, qualified through actual Native process scenarios. The actor-owned domain registry now captures revisions and names, provides CRUD/tags/Start/Stop/read views, retains snapshots and cleanup through independent TTLs, and is qualified through actual Native execution. The retained real API adapter now serves all15 M1 action paths over core, owns bounded preparation/response work and stable query-bound pagination, with actual Native computation through the typed HTTP layer. The opt-in main service now registers the actual target-family router first, validates supported YAML quotas, reports real worker/integration capabilities and retains a unique resumable API→grace→registry→callbacks→bridge shutdown before Lambda. Actual socket tests preserve modeled timeout envelopes and reject conflicting routing hints. Ordinary Rust SDK and boto3 now qualify the real strict-auth binary/Native path with17 independent semantic cases, all15 M1 actions and joined process/PID cleanup. Structured compilation, bounded Parallel/Map execution and registered nested start/.sync/.sync:2 and callbacks are now installed in the real service; terminal workflow persistence is installed; remaining release/platform requirements remain pending. B06 now closes the refreshed zstd advisories with one patched direct/transitive codec cohort, source-preserving loader overlay, old-frame/context/kernel regressions and clean fresh root audit/deny; preceding B01/B06 scans remain chronological evidence. The subsequent workflow/Native functional, persistence and current-host qualification requested first are implemented; work has now resumed on the preserved Docker-image checkpoint. Durable private image-pin intents/recovery are implemented and independently reviewed, with actual alias recovery and existing executor/pool/wire/container regressions on both retained daemons. W217 is now implemented under runtime§4.9/D76: immutable owner/engine binding, exact-lease runtime ports and a same-connection identity fence before each operation, with typed identity errors that cannot masquerade as resource404. Recorded namespace composition§4.10/D77 now drains containers before pins and audits untruncated owner containers plus complete bounded image-tag inventories; unrecorded resources block without adoption, and unknown records are never discharged by empty scans. Controlled pull preparation§2.14/D78 now adds exact registry/credential policy, bounded incremental progress and retained terminal ownership before image revalidation; D79 now supplies shared validated Docker budgets/keys, pure typed settings and retained private YAML credential loading. D80 adds shared validated endpoint/instance types, an owned HTTP(S) runtime probe, admission-independent gateway diagnostics and exact private endpoint receipts after physical retirement. D81 now consumes those receipts in the actual ordinary startup path, with a diagnostic-only same-listener phase and dependency-safe physical cleanup; controlled endpoint peers remain component evidence. Image/ZIP archives reject before import/backup retirement until the required immutable lease/ZIP consumers land. D82 adds independent custom Rust RIC, actual ImageConfig/immutable-revision and causal active-ID isolation qualification through ordinary startup. The remaining L01–L09 fault/admission matrix, handler SDK networking/callback images, IaC/reference/platform and final release acceptance remain required. The complete requested scope remains M0–M4. All feature documents below are in English; start here for the owned ASL engine and actual Docker image Lambda execution.

P3's explicit structured supervisor now owns asynchronous Parallel/Map calls, retained ancestor leases and per-frame causal history, including actual overlapping Native handlers. The real API now selects the registry-bound Structured profile with enforced YAML frame/result quotas, qualified by both ordinary SDKs through actual Native overlap and stopped nested handlers. Sequential library defaults remain available. The concrete atomic child registry port now has owned cancellation, typed envelope/depth/cycle bounds and actual Native child evidence. ASL child resources now use registered structured owners with exact parent-capability handoff, dispatch-free waiting and actual TaskSubmitted history. Both ordinary SDKs qualify nested envelopes and consumed Native handlers, including an actual single-dispatch server. The bounded callback token registry foundation now has secret-wrapped tokens, hash-only indexes, provisional completion, heartbeat timers and retained shutdown; Pure callback Parameters now use one evaluation with structural provenance, scoped zeroizing context and a distinct null-redacted history projection. The explicit observed Lambda callback port now binds unique report ownership and actual typed metadata, with provider-return timestamps, independent bridge settlement and actual Native early/late/failure/cancellation qualification. A deferred callback instruction assigns attempt/deadline before registered-token parameter preparation, tracks callback frames through retirement, validates heartbeat sources and projects typed Scheduled/Submitted history. The registered asynchronous owner now consumes this instruction with exact parent publication, bounded retained preparation/results, causal history acknowledgment, independent invocation/result/cleanup progress and actual Native ASL qualification. Explicitly installed runtime/Registry bindings admit callbacks; unregistered starts and services without a selected real Lambda port remain rejecting. The registry-bound public SendTask adapter is implemented under API§1.4/D67 with retained acknowledgment/error handling and documented empty success bodies; Main configuration/capability/shutdown composition and both handler-originated ordinary SDK paths now consume this binding under operations§1.7/D68. Both actual signed SDKs pass nine callback cases through Native handlers (20 consumed attempts total), including controlled early/late completion, heartbeat/timeout/retry and consumed PID retirement. The required SDK gate also proves process cleanup after fixture gate-release failure. B06 compression remediation is implemented/reviewed under operations§4.1/D69. The installed [bounded archive CBOR boundary](./rustack-workflows-60-operations-design.md#42-bounded-cbor-import-boundary) now passes full/minimal profile, actual DynamoDB/committed-generation and property qualification under D70. The [sealed registry export](./rustack-workflows-60-operations-design.md#31-sealed-registry-ownership-and-durable-export) now retains original ownership after clean shutdown and exports exact durable rows under a pre-copy budget, qualified through actual Native output/history and lifecycle/codec/clock failures. The [atomic library restore](./rustack-workflows-60-operations-design.md#32-startup-only-atomic-registry-restore) now validates, compiles and charges a startup-only replacement before one actor commit, preserving original history/retention and converting unexpected running records without replay. The [main workflow archive consumer](./rustack-workflows-60-operations-design.md#33-main-workflow-archives-and-startupshutdown-ownership) now installs this boundary in real named snapshots with retained startup/save ownership and actual Native process-restart/no-replay acceptance. The [pure workload and responsiveness gate](./rustack-workflows-70-verification-plan.md#51-native-host-pure-workflow-workload-and-responsiveness) now qualifies the actual current-host signed SDK/binary path. The [operator guide](../docs/workflows-guide.md) and [exact native resource inventory](./rustack-workflows-70-verification-plan.md#31-exact-resource-suites-and-documented-pure-smoke) expose the installed profile without claiming final release acceptance. Reference performance, consuming-project/IaC and final image/platform qualification remain open.

| Order | Specification | Type / purpose |
|---|---|---|
| 00 | [Workflows PRD](./rustack-workflows-00-prd.md) | Product goals, compatibility profile and complete M0–M4 scope |
| 10 | [API and data model](./rustack-workflows-10-api-design.md) | Smithy protocol, resource/revision identity, idempotency and execution history |
| 20 | [Lambda runtime](./rustack-workflows-20-lambda-runtime-design.md) | Image execution, typed operator settings, controlled pulls, container-origin endpoint proofs, Docker relay, revisions, pooling and cleanup |
| 30 | [ASL engine](./rustack-workflows-30-asl-design.md) | Scoped compiler, bounded frame scheduler and async resource owner, causal history, retries and deadlines |
| 40 | [Task integrations](./rustack-workflows-40-integrations-design.md) | Real Lambda invocation, nested `.sync`/`.sync:2`, callbacks and heartbeats |
| 60 | [Operations](./rustack-workflows-60-operations-design.md) | YAML configuration, budgets, topology, shutdown and snapshots |
| 70 | [Verification plan](./rustack-workflows-70-verification-plan.md) | Acceptance matrix, security boundaries, SDK/platform coverage and performance budgets |
| 80 | [Roadmap](./rustack-workflows-80-roadmap.md) | User-visible M0–M4 outcomes and calendar assumptions |
| 90 | [Implementation plan](./rustack-workflows-90-impl-plan.md) | P0 evidence gate and dependency-ordered P1–P5 tasks/exit criteria |
| 93 | [Implementation review](./rustack-workflows-93-review.md) | Independent findings, corrections and required phase/security follow-ups |
| 99 | [Key decisions and glossary](./rustack-workflows-99-decisions.md) | Alternatives, rationale, ownership and overloaded terms |

Read 00 → 10 → 20 → 30 → 40; apply 60/70 throughout. Read 80 for delivery planning and 90 for the next engineering step. The [supporting survey](../docs/research/survey-stepfunctions-lambda-container-runtime.md) records source verification and remaining runtime experiments.

The [pinned Docker transport study](../docs/research/study-bollard-docker-transport-boundaries.md) refines the P1 consumer boundary: reuse generated Docker types with bounded, supervised Unix HTTP/attach IO. It supersedes the initial ordinary Bollard pipe-client recommendation; it does not change the relay architecture or establish executor acceptance.

```text
┌─ P0: runtime experiment + semantic fixtures + pinned sources ───────┐
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼
┌─ M0 / P1: image Lambda ──────┐   ┌─ M1 / P2: API + ASL spine ─────┐
│ actual code + owned cleanup  │──▶│ real Lambda tasks + history      │
└─────────────────────────────┘   └──────────────┬───────────────────┘
                                                ▼
┌─ M2 / P3: Parallel/Map + nested .sync ──────────────────────────────┐
└─────────────────────────────┬──────────────────────────────────────┘
                              ▼ M3 / P4: callbacks + heartbeat
┌─ M4 / P5: ZIP + snapshots + consuming-project CI qualification ─────┐
└───────────────────────────────────────────────────────────────────┘
```

The new set supersedes conflicting historical Docker plans while preserving the hardening contracts. Ordinary Lambda invocation already waits for its result; `.sync` is implemented for nested Step Functions, not an invalid `lambda:invoke.sync` resource.

## 推荐阅读顺序

1. [系统审查与改进 spec](./rustack-system-review.md)：安全、正确性、架构及开发者体验；按 R01–R18 定位问题，按 M0–M4 安排修复。
2. 按问题查阅下表中的既有服务设计。Review 不整体替代服务设计，也不将设计中的未交付能力视为已经实现。
3. 快照相关工作按 PRD → 二进制归档设计 → 实施计划 → 验证计划阅读；早期 JSON 设计仅作历史背景。

```text
系统审查
  ├─ M0 安全边界 ── M1 有界执行、可靠停机 ── M2 数据语义
  │                                                    │
  └─ 既有服务设计提供协议与领域约束 ────────────────────┤
                                                       ▼
                                   M3 跨服务、配置契约 ── M4 体验与验收
```

## 跨系统与架构

| Spec | 类型 / 用途 |
|---|---|
| [rustack-system-review](./rustack-system-review.md) | Review：本次代码审查、风险排序、修复契约与验收场景 |
| [rustack-system-hardening-impl-plan](./rustack-system-hardening-impl-plan.md) | Impl plan：R01–R18全量修复、依赖顺序及验收门禁 |
| [rustack-runtime-hardening-design](./rustack-runtime-hardening-design.md) | Design：有界runtime、YAML、停机恢复、健康与能力 |
| [rustack-http-hardening-design](./rustack-http-hardening-design.md) | Design：认证、真实payload、HTTP预算与代理边界 |
| [rustack-lambda-hardening-design](./rustack-lambda-hardening-design.md) | Design：安全artifact、版本、执行准入及子进程生命周期 |
| [rustack-data-correctness-design](./rustack-data-correctness-design.md) | Design：R07/R08 DDB原子性与幂等、R09/R10 SQS、R15投递契约 |
| [rust-rewrite-feasibility](./rust-rewrite-feasibility.md) | Feasibility：LocalStack Rust 重写分析 |
| [smithy-codegen-all-services-design](./smithy-codegen-all-services-design.md) | Design：多服务 Smithy 代码生成 |
| [service-operations-gap-impl-plan](./service-operations-gap-impl-plan.md) | Impl plan：操作覆盖差距与分期 |
| [ruststack-pulumi-target-design](./ruststack-pulumi-target-design.md) | Design：Pulumi provider endpoint 集成 |
| [ruststack-pulumi-hackathon-app](./ruststack-pulumi-hackathon-app.md) | Validation：serverless 应用拓扑验证 |

## 服务设计

| 服务 | Specs |
|---|---|
| S3 | [早期实现](./ruststack-s3-implementation.md)、[Smithy 重设计](./smithy-s3-redesign-design.md)、[Checksum parity](./s3-checksum-parity-design.md) |
| DynamoDB | [设计](./ruststack-dynamodb-design.md)、[Streams](./ruststack-dynamodbstreams-design.md) |
| SQS | [设计](./ruststack-sqs-design.md)、[Long-poll / DashMap safety fix](./fixes/sqs-longpoll-dashmap-safety.md) |
| SSM | [Parameter Store](./ruststack-ssm-design.md) |
| SNS | [设计](./ruststack-sns-design.md) |
| Lambda | [Current runtime target](./rustack-workflows-20-lambda-runtime-design.md)、[Hardening](./rustack-lambda-hardening-design.md)、[服务设计](./ruststack-lambda-design.md)、[Executor](./ruststack-lambda-executor-design.md)、[Squib runtime](./ruststack-lambda-squib-runtime-design.md)、[S3 code packages](./ruststack-lambda-s3-code-design.md) |
| Step Functions | [PRD](./rustack-workflows-00-prd.md), [ASL engine](./rustack-workflows-30-asl-design.md), [integrations](./rustack-workflows-40-integrations-design.md), [implementation plan](./rustack-workflows-90-impl-plan.md) |
| EventBridge | [设计](./ruststack-events-design.md) |
| CloudWatch | [Metrics](./ruststack-cloudwatch-design.md)、[Logs](./ruststack-logs-design.md) |
| KMS | [设计](./ruststack-kms-design.md) |
| Kinesis | [设计](./ruststack-kinesis-design.md) |
| Secrets Manager | [设计](./ruststack-secretsmanager-design.md) |
| SES | [设计](./ruststack-ses-design.md) |
| API Gateway V2 | [设计](./ruststack-apigatewayv2-design.md) |
| IAM | [设计](./ruststack-iam-design.md) |
| STS | [设计](./ruststack-sts-design.md) |
| CloudFront | [Management plane](./rustack-cloudfront-design.md)、[Data plane](./rustack-cloudfront-dataplane-design.md) |

## Runtime Snapshot

| Spec | 类型 / 用途 |
|---|---|
| [ruststack-snapshot-prd](./ruststack-snapshot-prd.md) | PRD：用户可见的命名快照契约 |
| [ruststack-snapshot-design](./ruststack-snapshot-design.md) | Design：早期 JSON 布局、服务边界与生命周期背景 |
| [ruststack-snapshot-binary-archive-design](./ruststack-snapshot-binary-archive-design.md) | Design：当前二进制归档目标 |
| [ruststack-snapshot-impl-plan](./ruststack-snapshot-impl-plan.md) | Impl plan：依赖顺序与实施阶段 |
| [ruststack-snapshot-verification-plan](./ruststack-snapshot-verification-plan.md) | Verification plan：快照测试与验收 |
