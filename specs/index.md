# Rustack Specs 索引

本文件是 specs 的导航入口；文件中的 Draft / Implemented 状态以各原文为准，不代表经过本次运行验收。

## Step Functions and Lambda Runtime — new specification set

Status: implementation contract dated 2026-10-25; P0 evidence and P1 invocation/artifact/cancellation plus shared Native/relay/client runtime contracts are implemented. Bounded Docker management/attach IO, platform-aware local resolution and owned image preparation/leases are also implemented and qualified against real RIC images. Typed deployment settlement, leased invocation capture and retained provider deployment ownership/API wiring are implemented. Explicit side-effect-free code DryRun and strict bounded deployment decoding are implemented and independently reviewed with live SDK/HTTP validation evidence. The authentication/shared-core/Lambda production boundary strict gate is also implemented and independently reviewed. B01 dependency security is closed with patched libraries, explicit regional SDK features and ZIP9 regressions; audit and all deny checks pass. Persistent descriptor-backed directory ownership and pin/cleanup retention are implemented and independently reviewed (§4.1/D28); prior-run reconciliation remains pending. Independent bounded function log capture and output-only live attach are also implemented/reviewed (§3.7/D29), with actual stdout/stderr receipt in both real wire suites. Provider control-plane tests use an explicit test executor capability; production Docker remains unavailable until the Docker executor/probes land. The ASL engine also remains pending; see the implementation ledger for verified scope. All documents in this set are in English. Start here for the requested ASL engine and actual Docker image Lambda execution.

| Order | Specification | Type / purpose |
|---|---|---|
| 00 | [Workflows PRD](./rustack-workflows-00-prd.md) | Product goals, compatibility profile and complete M0–M4 scope |
| 10 | [API and data model](./rustack-workflows-10-api-design.md) | Smithy protocol, resource/revision identity, idempotency and execution history |
| 20 | [Lambda runtime](./rustack-workflows-20-lambda-runtime-design.md) | Image resolution, Docker relay/runtime, immutable revisions, pooling and cleanup |
| 30 | [ASL engine](./rustack-workflows-30-asl-design.md) | Compiler, JSONPath data flow, eight state types, retries and deadlines |
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
