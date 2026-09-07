# Rustack 系统修复实施计划

状态：实施中。范围：[系统审查 R01–R18](./rustack-system-review.md)，所有里程碑均为本次交付范围，不以 P0 修复代替全部完成。

## 1. 完成定义

每个 R 项同时交付代码、针对原失败链的回归、必要的配置/能力说明。保留 AWS wire 的大小写/协议；禁止以禁用测试、吞错、只改 README 为方式关闭已有领域行为缺陷。安全默认对 CLI 的兼容变化需明确迁移说明。代码提交不包含用户已有的工具链删除或无关文件。

工程规范以 [AGENTS.md](../AGENTS.md) 为准。实施前各组件将 review 细化为设计契约并更新本索引；设计可补充选择，但不能降低原不变量。用独立 reviewer 对最终 diff 逐项核查，不以分工作者自检替代。

## 2. 共同决定

- **D-H1**：默认 CLI bind 改为 `127.0.0.1:4566`；Lambda 默认 Disabled（不能默默 echo 成功），native/auto 需 operator 显式选择。Docker 如监听全网是容器内部 networking 需要，宿主示例端口仅发布到 loopback，并明示风险。
- **D-H2**：严格认证必须有 provider。保留各协议现有配置入口以减少无关 public API 变更，但归一化为内部 AuthMode，非法组合在启动和每个 HTTP 服务边界均拒绝；不能只靠 main 启动检查。
- **D-H3**：普通 SigV4 校验实际正文摘要；S3 特殊签名协议有独立受限策略，不能扩散到 JSON 服务。日志不记录 Authorization 或预期签名。
- **D-H4**：HTTP proxy 不跟随重定向、不继承环境 proxy；原 30x/Location 转发给 viewer。仅按 operator 配置的 origin/integration 连接，允许本地模拟器明确配置的 HTTP fixture（记录为相对 AGENTS.md 默认 https-only 的本地集成例外），不增加任意 request 指定上游的能力。
- **D-H5**：新代码目录不可变且与逻辑名称解耦。验证完 ZIP 再发布 artifact 引用；发布版本复用不可变 artifact，而非 latest 目录。warm key 包括 artifact/config revision。
- **D-H6**：runtime supervisor 的成员不由 snapshot 支持与否决定。接受工作前获取有界容量，quiesce 先于 snapshot，最终终止在保存之后。不能清空队列再保存。
- **D-H7**：快照发布采用可恢复的提交协议；正常新目录布局与现有 snapshot 可读性保留，不引入 WAL。恢复必须显式，不能把存在完整旧 backup 的情况解释为初次空启动。
- **D-H8**：Data APIs 的原子性及 token 窗口在状态所有者实现；不能仅为 transaction 请求加锁、让普通写入绕过。DLQ handoff 通过 manager/actor 协议，不跨 actor 相互等待。

## 3. 公共预算与兼容规则

预算均为有效配置的一部分，使用非零数值和上限校验；环境覆盖 YAML，显式坏值不得 fallback。初始默认值：256 个已接入 HTTP connection、128 个业务 in-flight、5 秒读头、30 秒控制面请求、30 秒上游、30 秒总停机；SQS 长轮询可正常完成。小控制面默认最多 16 MiB，Lambda ZIP JSON envelope 最多 96 MiB，Lambda Invoke 仍按模式/服务限制；S3 对象输入按 5 GiB 上限有界 streaming，XML/控制面不得借此变成 5 GiB collect。上游响应默认最多 64 MiB，必须逐帧计数而非读后检查。body idle/总 deadline 与 HTTP/2 streams 同时受准入约束。健康/诊断小响应最多 64 KiB，探针总期限 3 秒。

Lambda 默认全局并发 32、每函数并发 8、全局空闲实例最多 32、每函数空闲最多 1；显式 reserved concurrency 覆盖函数默认且 0 拒绝调用。异步 Event 队列/工作集合容量 128；返回 202 前必须占用容量，任务成功/错误/取消均记录，退出必须等待或在预算内取消。最终数值属于保守安全默认，不声称代表经性能调优的吞吐上限。

快照只保留已声明持久化的服务/状态。所有已接受后台工作都受监督，但 SQS 消息和不支持持久化的服务并不因此获得跨重启保证；输出包含/遗漏摘要。能力状态为 implemented/partial/metadata-only/unsupported，健康的 liveness 与 readiness 分开。

## 4. 任务及里程碑

| 阶段 / 任务 | R 项 | 交付 | 退出测试 |
|---|---|---|---|
| M0.1 认证边界 | R02/R03 | 内部 AuthMode、所有验签入口 fail closed、实际摘要校验、脱敏 | 缺凭证/坏签名/篡改/重复哈希头 handler 不执行 |
| M0.2 artifact 安全 | R01 | 私有有效名称、ARN/qualifier 解析、受控物理路径和 symlink 防护 | 临时哨兵在恶意 CRUD/导入后不变 |
| M0.3 proxy 边界 | R04 | 禁止 redirect follow/environment proxy、规范 URL | 各类 30x 的第二上游请求数为零 |
| M1.1 IO 预算 | R05 | collect 前限制、流式计数、连接/请求限额、deadline | 超额和慢 body/upstream 在预算内退出且资源释放 |
| M1.2 Lambda 工作管理 | R06 | 有界 Event、执行 permit、reserved concurrency、warm 总量 | N+K 并发不超过 N、reserved=0、退出无遗留任务 |
| M1.3 supervisor / snapshot | R13/R14 | SIGINT/TERM、quiesce/drain/save/stop、有期限、恢复协议 | 真实子进程信号与发布步骤失败注入 |
| M2.1 DDB 事务与 token | R07/R08 | prepare/commit 隔离、提交后 stream、有界10分钟幂等状态 | 坏后项全回滚、竞争事务、读一致性、满缓存重放 |
| M2.2 SQS | R09/R10 | 可恢复 DLQ manager、结构化 dedup key | standard/FIFO 阈值、目标异常不丢、跨组碰撞回归 |
| M2.3 Lambda 发布 | R11/R12 | staging ZIP、传播错误、不可变版本、warm revision | A/B 冷热版本不变量、所有失败更新保旧 |
| M3.1 bridge | R15 | 完整 SqsParameters、明确 unavailable/unsupported、受监督投递 | FIFO group roundtrip/消费、缺依赖不假成功 |
| M3.2 config | R16 | 统一 YAML+env 校验、有效配置、bind/advertised endpoint | 拼写/范围/冲突拒绝，非默认端口/IPv6 URL |
| M4.1 health | R17 | 状态/能力 registry、GET/HEAD readiness、有界结构化探针 | 假响应/永不EOF/失败依赖/限额压力 |
| M4.2 developer UX | R18 | 修正 package/bin 示例、能力/快照声明、唯一目录索引 | 文档 walkthrough、链接、示例操作回归 |

独立领域可并行实施，但合并时以上共享契约优先。每个组件设计记录实际结构/API，不能用本表的组件名替代实现设计。

## 5. 状态/生命周期

```text
┌─ Validated runtime ──────────────────────────────────────────────┐
│ cfg/auth/budgets ──► Gateway admission ──► protocol adapters       │
│                          │                      │                │
│                          └────► Runtime supervisor ◄─────────┐  │
│                                      │                       │  │
│ bounded jobs ──► Lambda / Events / SNS / SQS handoff workers ──┘  │
│                                      │                          │
│ signal ──► stop ingress ──► quiesce ──► snapshot ──► stop        │
└─────────────────────────────────────────────────────────────────┘
```

拒绝/错误不能触发后续业务变更；accepted 与 delivered 分开；先校验后入队，先持有 capacity 后返回接受。snapshot 出错返回非成功退出并保持上一份可恢复状态。

## 6. 质量门禁与交付

1. 每任务 targeted unit/integration 回归；仅使用临时目录、loopback fixture、无真实账号请求。
2. 完成后运行现有 Rust build、test、nightly fmt、strict clippy，含 workspace/all-targets；运行 rustdoc broken-link 检查与必要 feature 组合、边界 lint。
3. 依赖变化运行 cargo audit/deny，不能忽略失败或无依据消除诊断。
4. 独立代码审查按 R-ID 给证据；有效缺陷全部修复再复验。
5. 命名路径逐项 stage，保留用户其他改动；向现有 origin 推送独立分支并创建 PR，PR body 给每项实现/测试索引、兼容变化和任何真实未通过的外部条件。只有完成全部 R 项才使用 completed 声明。

## 7. 追踪入口

原始证据：[rustack-system-review.md](./rustack-system-review.md)。设计与验证证据通过 [index.md](./index.md) 导航；实现过程产生的新发现继续追加到原 review 的独立后续段落，不丢在聊天中。

## 8. 实施与验证记录（分支 fix/system-review-r01-r18）

状态：R01–R18 全部交付并通过验证。本段如实记录已执行证据与剩余外部条件，不改写原审查段落。

### 8.1 交付范围

- M0：`rustack-auth` 内部 `AuthMode`，19 个 HTTP adapter fail closed；普通 SigV4 校验实际正文摘要；S3 流式/UNSIGNED 收窄策略与 trailer 校验；CF/APIGW proxy 无 redirect/无环境 proxy；Lambda 标识/路径/ARN 校验与不可变 artifact、发布版本引用、warm 指纹。
- M1：`rustack-core::http` BodyBudget/BudgetedBody/collect_body；网关连接+业务信号量与 HTTP/2 流预算、响应体结束才释放；DDB permit 移入 spawn_blocking；Lambda 32/8/128/reserved=0/Event 1 MiB；显式 `Runtime` + `shutdown_timeout(0)`。
- M1.3：`RuntimeWorkers` supervisor 顺序 quiesce→snapshot→并行 stop；SIGINT/TERM；快照排他锁、fsync staged、`.name.previous` 恢复、失败不空启动。
- M2：DDB operation gate prepare/commit、提交后 stream、10 分钟 token SHA-256（满额不淘汰有效项）；SQS manager/actor DLQ 移交、FIFO 结构化 dedup；Events/SNS unavailable/unsupported、监督投递与统计。
- M3/M4：`settings` YAML+env 校验与 `RUSTACK_CONFIG`/`RUSTACK_ADVERTISED_ENDPOINT`；GET/HEAD health + `/_health/{ready,live}` + `/_rustack/capabilities`；有界结构化探针；README/Makefile/Dockerfile/docs 能力与安全指南。
- 依赖修复：quick-xml 0.39→0.41（RUSTSEC-2026-0194/0195）、h2 0.4.13→0.4.19（RUSTSEC-2026-0258）、quinn-proto→0.11.17、anyhow→1.0.104（RUSTSEC-2026-0190）、chacha20/spin 去 yank。
- 独立审查发现的 P2 全部修复：SQS 毒消息不阻塞健康消息（skip 失败项继续扫描）、SNS 下游 panic 隔离与 Subscribe 幂等容量顺序、Events unsupported 参数显式建模并拒绝、CF `unsafe map_unchecked_mut` 消除并 forbid unsafe、测试 wire 构造缺失尾部 CRLF 修复。

### 8.2 验证命令与结果（全部在显式 `+1.98.1` 工具链、共享 workspace target 下运行）

- `cargo +1.98.1 check --workspace --all-targets --all-features`：通过。
- `cargo +1.98.1 test --workspace --all-features --exclude rustack-integration`：119 组 1683 项全部通过；DDB/SQS/Events/SNS 域 393+，S3/HTTP 105，Lambda core 143 + 原生 A/B warm/Event 收割 fixture。
- `cargo +nightly fmt --all -- --check`：通过。
- `cargo +1.98.1 clippy --workspace --all-targets --all-features --no-deps -- -D warnings -A renamed-and-removed-lints -A unused_async -A clippy::unused_async -A clippy::unused-async-trait-impl -A clippy::result_large_err`：通过。允许的两类存量 lint（大量既有 async 无 await 的 handler、模型 Err 大类型）在基线中已存在且与本次领域行为无关，另两个 allow 仅为 lint 改名兼容；不掩盖本次新增代码问题（新增文件/改动已逐条修复至零告警）。
- `cargo audit`：exit 0（仅 aws-sdk-s3 dev/test 传递的 lru 0.16.4 unsound 警告，修复需 sdk≥1.145，超出仓库保留工具链；生产路径不包含）；`cargo deny check`：ok。
- 真实进程冒烟（debug 二进制 + `--snapshot devsmoke`）：health 200、capabilities 结构化、DynamoDB CreateTable/PutItem → SIGTERM → 同快照重启 GetItem/ListTables 数据恢复；退出码 0。
- rustack-cli bin 单测新增：CLI 严格解析冲突、settings 解析失败矩阵、health probe 拒假状态/无 EOF/超限、快照恢复/排他锁/publish 保留旧代际、S3 17 MiB 流式 spool 与签名 trailer 篡改、events 注入 worker 崩溃 readiness 等。

### 8.3 诚实边界（非代码缺陷但应公开）

- 未修改/不恢复用户删除的 `rust-toolchain.toml`；仓库提交的固定工具链沿用基线值（本次全部验证用显式 `+1.98.1`）。
- 全 workspace pedantic 存量 lint（上表 allow）与 dev-only lru 警告不随本 PR 修复，原因如上。
- Dockerfile 重写为 BuildKit 缓存/`--locked`/scratch 非 root，本机无 docker 未执行镜像构建；rust:1.94-slim 基镜像仅提供构建工具链，最终产物无工具链依赖。
- `rustack-integration`（需运行中 server 的 aws-sdk 套件）未纳入默认 workspace 运行；真实进程冒烟已覆盖代表性 s3+dynamodb 路径。
