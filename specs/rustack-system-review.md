# Rustack 系统审查与改进 Spec

- **类型**：Review / 修复建议与验收契约
- **日期**：2026-09-07
- **状态**：静态审查完成；修复方案待实施，运行验收未执行
- **基线**：`03599661f0a4c7cd2937e90691851fc65a5496df`，以审查时工作区源码为准
- **范围**：安全、数据正确性、生命周期、跨服务架构、CLI / SDK / IaC 开发者体验
- **入口**：[specs/index.md](./index.md)

> **修复与验收后续**（2026-09-07）：R01–R18 已在分支 `fix/system-review-r01-r18` 实现并通过验证。原审查发现与建议段落保持为历史结论不变；实现决策见四个 hardening 设计，逐项实现/测试/已知边界见 [实施计划 §8 实施与验证记录](./rustack-system-hardening-impl-plan.md#8-实施与验证记录分支-fixsystem-review-r01-r18)。

## 1. 目的与结论

Rustack 的核心价值不是复刻全部 AWS，而是让开发者在本地及 CI 中快速、可信地验证 AWS 应用。本次审查关注两类损失：恶意或错误输入影响运行 Rustack 的机器，以及本地“成功”掩盖真实的数据、投递或部署错误。

代码已有清晰的 `model / http / core` 分层、跨服务 trait bridge、SQS actor、部分资源校验和流式解压上限。优先问题不是拆更多 crate 或换更多依赖，而是补齐**边界验证、原子提交、有界执行、生命周期和能力声明**这些跨模块契约。

审查收敛为 **18 项**。最高优先级是 Lambda 路径越界、严格验签失效和 SigV4 正文完整性；其次是事务部分提交、消息不可达、Lambda 版本失真与可靠停机。不能把当前实例当作生产 AWS、IAM 授权测试平台或安全的多租户执行环境。

### 1.1 方法与可信度

- 沿 HTTP → handler → provider → storage / executor / bridge 调用链核查，而不是按 `unwrap`、`spawn` 搜索数量推断漏洞。
- 深查 gateway、auth、Lambda、DynamoDB transactions、SQS、EventBridge/SNS bridge、CloudFront/APIGW HTTP proxy、snapshot；其他服务的协议层和配置仅抽样。**不是全部 18 个服务、所有生成模型和算法的穷尽审计。**
- 查阅根目录 [AGENTS.md](../AGENTS.md)、现有 specs、`docs/research` 的 11 篇 memo 的相关章节；`vendors/localstack` 当前为空，未初始化 submodule 或把旧研究当作本次实测。
- 问题中的代码行为均有静态依据；“触发 / 验收”是待实施的回归场景，**没有实际发送攻击请求、破坏文件、执行不可信 Lambda 或进行压测**。并发时序、进程退出与重定向影响仍需隔离环境运行确认。
- 本次只修改 specs 文档，不修改执行逻辑、Rust 构建 / 测试 / 工具链契约。Rust build/test/fmt/clippy 验证不适用；不宣称 cargo audit 全量通过。
- 开始时已有 `rust-toolchain.toml` 删除和 `.DS_Store` 未跟踪状态；均保留，不归因于代码缺陷，也不恢复用户改动。

### 1.2 优先级

| 级别 | 含义 |
|---|---|
| P0 | 外部输入可越过文件或认证完整性边界；共享 / 非可信网络使用前必须处理 |
| P1 | 高影响资源耗尽、数据错误、消息丢失或关键部署语义失真；稳定性里程碑必须处理 |
| P2 | 有限条件下的兼容性、可诊断性、体验问题；不能长期用“本地工具”解释 |

这里的 P0–P2 是工程优先级，不是 CVSS。风险随网络可达性、进程权限、执行器和部署方式变化。

## 2. 信任边界与非目标

```text
外部 / 非可信输入
┌─────────────────────────────────────────────────────────────────────────┐
│ AWS SDK / CLI / IaC   HTTP body / headers   Lambda ZIP   上游 HTTP 30x   │
└──────────────┬───────────────────────────────────────────────┬──────────┘
               │                                              │
┌──────────────▼──────────── Rustack 进程 ──────────────────────┼──────────┐
│ Gateway: 路由 / 限流 / 健康 / 生命周期                        │          │
│       │                                                       │          │
│       ▼                                                       ▼          │
│ 各服务 HTTP + Auth ──► 已验证领域命令          CF / APIGW HTTP client      │
│       │                           │                           │          │
│       ▼                           ▼                           │          │
│ DDB 数据状态                 SQS queue actor ◄─ SNS / Events bridge      │
│ 事务 / 索引 / Streams         队列 / 去重 / DLQ                 │          │
│       │                           │                           │          │
│       └──────────► SnapshotService registry                    │          │
│                            │                                  │          │
│ Lambda provider ──► 代码包存储 ──► Executor / warm pool         │          │
└────────────────────────────┼─────────────────┼────────────────┼──────────┘
                             ▼                 ▼                ▼
                    宿主文件系统 / 快照    native 子进程     指定上游及内网
                                          或 Squib VM       必须限制目的地
```

**部署提醒**：`main.rs:815-816` 默认监听 `0.0.0.0:4566`；默认跳过验签；`LambdaConfig::from_env` 默认 `auto`（`config.rs:54-65`），非 macOS 的 auto 走 native（`executor/auto.rs:40-51`）。native 直接执行 bootstrap（`executor/native.rs:100-119`），清空继承环境并不隔离文件系统、网络和进程权限。这个组合意味着：可访问 API 的调用者可提交以 Rustack 权限运行的代码。

这不是 shell 注入或已证明的 VM 逃逸，native 无沙箱也是[既有执行器设计](./ruststack-lambda-executor-design.md)的明确非目标。但默认暴露面应收紧：安全默认监听 loopback、执行用户代码需显式选择可信 native 模式；容器对外发布端口需 operator 主动确认。过渡期至少禁用不需要的 Lambda、使用隔离测试账号/主机和最小文件权限，勿上传真实秘密。**`LAMBDA_EXECUTOR=disabled` 只关闭执行，不能阻止 R01 的代码存储越界。**

其他非目标：不要求本轮实现生产 IAM、全部 AWS 服务、跨账户隔离、WAL 或完整服务持久化。不把“支持 metadata”当成“支持执行”；可以只模拟配置，但必须显式披露。

## 3. 问题总表

| ID | 优先级 | 问题 | 主要影响 |
|---|---|---|---|
| R01 | P0 | Lambda FunctionName 未限制路径语义 | 代码根外写入、递归删除 |
| R02 | P0 | 严格验签缺凭证时 fail-open | 未签名请求仍执行 |
| R03 | P0 | SigV4 声明哈希覆盖实际正文哈希 | 有效签名可搭配被篡改正文 |
| R04 | P1 | HTTP proxy 默认跟随重定向 | 有前提 SSRF、代理语义变化 |
| R05 | P1 | 请求 / 上游正文及连接缺少有效全程预算 | 内存、连接、任务资源耗尽 |
| R06 | P1 | Lambda warm 限额不限制 in-flight / Event | 进程、FD、后台任务无界增长 |
| R07 | P1 | DynamoDB 事务非原子、缺隔离 | 失败仍提交、条件竞争、撕裂读取 |
| R08 | P1 | DynamoDB 忽略 ClientRequestToken | 重试重复业务更新 |
| R09 | P1 | SQS DLQ 只移动到内部不可读 Vec | 消息从用户视角消失 |
| R10 | P2 | FIFO messageGroup 去重键有歧义 | 跨组错误去重 |
| R11 | P1 | Lambda ZIP 更新非原子且吞解压错误 | 错误成功、旧代码被破坏 |
| R12 | P1 | Lambda 发布版本共享可变代码目录 / warm key | 版本执行错误、更新后仍执行旧代码 |
| R13 | P1 | 停机只处理 Ctrl+C，drain 无期限 | 容器退出不保存、挂起停机 |
| R14 | P2 | Snapshot 双 rename 中断无恢复 | 已完成旧快照被当成不存在 |
| R15 | P1 | 跨服务投递契约缺参数、缺能力失败语义 | FIFO 投递失败、Noop 假成功 |
| R16 | P2 | 配置静默回退、URL 与监听不一致 | 错配置难发现、生成错误入口 |
| R17 | P2 | 健康是注册状态，探针无界且用子串判断 | 假健康、探针卡住 |
| R18 | P2 | 开发入口和能力说明不可靠 | 启动失败、本地验证误导 |

## 4. 安全与资源问题

### R01 — Lambda 函数名可越过代码目录边界

**证据**：[provider.rs](../crates/rustack-lambda-core/src/provider.rs) `311-316` 只检查 FunctionName 长度；`440-442` 将其传给 `process_code`。[storage.rs](../crates/rustack-lambda-core/src/storage.rs) `954-977` 用 `code_dir.join(function_name).join(version)` 写 `code.zip` 并删除既有 `extracted`；`1014-1017` 清理时递归删除 `code_dir.join(function_name)`。[router.rs](../crates/rustack-lambda-http/src/router.rs) `70-73` 会解码路径参数，provider `767-791` 删除记录后调用清理。

**触发 / 影响**：可调用 Lambda CRUD 且进程对目标可写时，创建名为绝对路径或 `../...` 的函数，会写到根外 `<目标>/$LATEST/`；再以编码后的名称删除，可删除已登记函数对应的整个目标目录。无需 Invoke，disabled executor 也不防护。这里不是说任意单次 DELETE 都能删除不存在的函数：删除要求记录存在。

**已有防护**：ZIP entry 的 `enclosed_name` 约束的是内层文件，不能证明外层 extraction root 安全。

**修复契约**：名称/ARN/qualifier 分别解析成私有字段 newtype；逻辑名称不得直接充当磁盘路径，使用内部不可控 ID / 内容摘要布局。所有 CRUD、版本、快照导入共用边界校验；拒绝路径分隔符、绝对路径、父目录组件，并防预存 symlink。按 AGENTS.md § Input Validation / Path traversal 执行，不能只做字符串替换。

**验收**：仅在测试临时目录设置根外哨兵；绝对路径、`..`、编码斜杠、symlink root 的创建/更新/删除均失败，哨兵与原文件树不变；合法函数和合法 ARN 引用仍正常。

### R02 — 配置要求验签，却因缺凭证继续放行

**证据**：[main.rs](../apps/rustack/src/main.rs) `608-616` 缺任意一项凭证返回 `None`；`395-401` 仍构造 SQS HTTP config。[SQS service.rs](../crates/rustack-sqs-http/src/service.rs) `128-145` 使用 `!skip_signature_validation` 与 `Some(provider)` 两层 if，缺 provider 时直接 dispatch。

**触发**：设置 `SQS_SKIP_SIGNATURE_VALIDATION=false`，但 ACCESS_KEY / SECRET_KEY 及 AWS fallback 缺两者或缺一者；无签名合法 CreateQueue 仍能到业务层。与“默认开发模式跳过验签”的有意设计不同，这是显式安全配置未被履行。此条有完整 SQS 证据，不据此假定每个服务行为完全相同。

**修复契约**：严格模式凭证缺失在监听前报错；HTTP 层也 fail closed。把非法组合改为 `AuthMode::Required(provider)` / 显式开发模式，并记录启动时认证模式而非秘密。

**验收**：false + 缺任意凭据无法启动 / 业务 handler 调用数为 0；完整凭据下有效签名通过、无签名/坏签名拒绝。扫描其他协议实现同一模式，不能仅修 SQS。

### R03 — SigV4 正文可以与签名脱钩

**证据**：[SQS service.rs](../crates/rustack-sqs-http/src/service.rs) `122-145` 计算真实 body hash，但 [sigv4.rs](../crates/rustack-auth/src/sigv4.rs) `249-256` 无条件优先使用请求声明的 `x-amz-content-sha256`；`258-286` 用旧声明值重算签名而不核对实际正文。

**触发 / 前提**：拥有一份合法已签 SQS SendMessage 请求，不需要 secret；保留签名、路径和签名头，将 MessageBody 改为同长度内容并保持旧哈希头，仍会按旧哈希验证。即使原请求没带哈希头，也可新增未列入 SignedHeaders 的该头，值取原正文哈希，造成同样脱钩。严格验签且凭据完整也受影响。

**已有防护 / 限定**：HMAC 使用 constant-time 比较是正确的；普通非流式 S3 HTTP 层有额外正文校验，不能直接推断该 SQS 调用链同样攻破全部 S3 路径。

**修复契约**：普通 SHA-256 声明必须格式合法且等于实际字节摘要；不能把“客户端签了某个哈希”当成“正文就是该哈希”。`UNSIGNED-PAYLOAD` / `STREAMING-*` 等占位符由明确协议策略处理，流式场景须校验真实 chunk / trailer 完整性，禁止为兼容 S3 而放宽所有 JSON 服务。

**验收**：有效请求通过；保持签名并改 1 字节正文拒绝且 handler 未调用；覆盖新增未签名哈希头、无哈希头、格式错误、重复头与允许的 S3 流式用例。

### R04 — CloudFront / APIGW 自动跟随上游重定向

**证据**：[CF plane.rs](../crates/rustack-cloudfront-dataplane/src/plane.rs) `84-91` 仅设置 timeout，未设 redirect policy；[dispatch.rs](../crates/rustack-cloudfront-dataplane/src/dispatch.rs) `248-270,284-305` 请求上游并回传最终响应。[APIGW provider.rs](../crates/rustack-apigatewayv2-core/src/provider.rs) `49-54` 使用默认 Client；[http_proxy.rs](../crates/rustack-apigatewayv2-core/src/execution/http_proxy.rs) `22-64` 同样请求并返回最终正文。reqwest 默认会跟随重定向。

**触发 / 前提**：攻击者控制已经配置的上游响应，或上游有路径型 open redirect；合法 viewer 请求匹配路由 / CF cache miss，上游返回 Location 指向 Rustack 可访问的其他 loopback、私网或 link-local 服务。代理可能回传该服务内容；CF 还可能缓存结果。不假定攻击者可拿到 IMDS 凭据，也不假定未转发的 query 能触发 redirect。

**设计区分**：主动配置 localhost origin 是本地模拟器的功能，不独立判漏洞；问题是**上游响应可把选定目的地扩展到另一个服务**。已有 CF timeout、method 检查、hop-header 过滤不限制重定向目的地。

**修复契约**：默认不跟随，转发原始 30x / Location。若存在需要跟随的显式模式，operator egress policy 对初始 URL、更新后的 URL 和每跳 URL 校验 scheme、host/IP、port，检查所有 A/AAAA 并绑定获准解析结果，处理代理设置与 DNS rebinding。生产式硬化模式遵循 AGENTS.md § URL / SSRF；本地 HTTP fixture 例外必须显式、精确且被记录，不能泛放所有内网地址。

**验收**：A 上游返回 301/302/303/307/308 指向未授权 B；B 请求数为 0，viewer 收到原 30x 或明确错误，B 正文不得进入 CF cache。覆盖相对/绝对/协议相对 Location、不同端口、IPv6/mapped address、DNS 公转私。

### R05 — 资源上限未贯穿整个 IO 生命周期

**证据**：[main.rs](../apps/rustack/src/main.rs) `640-659` 每个 accept 都 spawn，没有应用级连接准入预算；[SQS service.rs](../crates/rustack-sqs-http/src/service.rs) `122-133,151-157` 在验签前直接 `.collect()`；[CF dispatch.rs](../crates/rustack-cloudfront-dataplane/src/dispatch.rs) `287-297` 先 `.bytes().await`，完整分配后才检查 `max_body`；[APIGW http_proxy.rs](../crates/rustack-apigatewayv2-core/src/execution/http_proxy.rs) `45-55` 完整收集上游响应。

**触发**：合法操作头 + 超大/持续 chunked body，或上游持续输出、大量慢连接。单个业务字段上限、Content-Length 校验、CF 读后大小检查，都不能限制实际收集阶段的总占用。此结论是应用预算缺失，不声称 Hyper 自身没有任何协议默认限额。

**修复契约**：统一定义连接、in-flight、正文实际字节、读头/读体/上游 deadline；JSON 控制面可在 collect 前使用 `http_body_util::Limited`，S3 数据面保留有界 streaming，不能把一个过小全局 body limit 粗暴套到对象上传。上游逐帧计数并超限中止，限制聚合内存；超额返回协议对应错误。

**验收**：预算 B 下，B+1 实际字节即失败；无 Content-Length/chunked 不能绕过；慢请求按 deadline 结束；并发 N+1 不产生无界任务；健康探针在压力下仍有独立资源可用。RSS / FD / 任务峰值记录预算内断言，不能只断言最后返回 413。

### R06 — Lambda 并发限制只管空闲池

**证据**：[provider.rs](../crates/rustack-lambda-core/src/provider.rs) `872-887` 每次 Event detached spawn；[instance.rs](../crates/rustack-lambda-core/src/executor/instance.rs) `117-122` 无空闲实例就新建，`186-195` 仅在归还时检查 `max_warm`，`199-211` 新建 Runtime API 和 backend。PutFunctionConcurrency 保存的 metadata 未参与这条执行准入路径。

**影响**：大量慢 Invoke / Event 可创建大量 native 进程、socket、任务。外层 HTTP 并发限额无法覆盖已返回 202 的 Event。已有 invoke timeout、kill_on_drop、4 KiB 日志环并不是全局并发预算。

**修复契约**：入队前获取全局与每函数执行配额，reserved concurrency=0 明确拒绝；Event 使用有界队列并由 supervisor 管理。permit 覆盖排队/执行的明确阶段，异常、超时和取消必须释放。区分 `maxIdleInstances` 与 `maxConcurrentInvocations`，限制全局 warm 总量；native 的 OS 资源控制不能用请求内存 metadata 冒充。

**验收**：设置并发 N，发起 N+K 同步和异步慢调用，运行实例至多 N，其余明确拒绝或有界排队；返回 202 必须已获队列容量；异常/取消/停机后无 permit 泄漏、后台工作可收拢。

## 5. 数据与执行正确性

### R07 — DynamoDB 事务失败后仍留下前序写入

**证据**：[provider.rs](../crates/rustack-dynamodb-core/src/provider.rs) `2455-2479` 预检 condition，`2481-2509` 立即写入并发出 stream；`2549-2558` 才解析/求值 UpdateExpression，错误直接退出。条件检查和提交之间也无整体隔离；事务读是逐项取值。

**最小场景**：空表 T，主键 pk:S；同一 TransactWriteItems 先 Put(pk=a)，后 Update(pk=b, UpdateExpression="SET")，均无条件。第二项解析失败，但 a 已写入；开启 Streams 时还有前序记录。并发客户端做 `attribute_not_exists` 条件事务，也可能均通过预检。

**修复契约**：prepare 阶段完成全部表达式、类型、大小、主键和条件计算，形成暂存变更；单一事务协调状态所有者负责隔离、原子提交与提交后 stream 发布。普通写入、读和事务必须参与同一协调协议，不能只串行化 TransactWriteItems；禁止用“多张表各自 DashMap”冒充事务。

**验收**：上述坏 Update、运算失败、后序非法主键均不留下前序数据 / stream；用 barrier 控制两个竞争条件事务，至多一个成功；TransactGetItems 不读到新旧混合。现有失败 ConditionCheck 测试不能替代提交阶段失败测试。与[既有操作差距计划](./service-operations-gap-impl-plan.md)中的事务原子性要求一致。

### R08 — DynamoDB 事务重试没有幂等语义

**证据**：[model/input.rs](../crates/rustack-dynamodb-model/src/input.rs) `585-587` 接受 `client_request_token`，但 [provider.rs](../crates/rustack-dynamodb-core/src/provider.rs) `2384-2594` 的事务实现不使用该字段；core 源码搜索无该字段命中。

**场景**：初始 n=0；使用相同 token、相同 Update `ADD n :one` 连续提交两次，每次都会重新执行，结果 n=2。修复 R07 并不会自动修复重试。

**修复契约**：按已有 spec 的 10 分钟窗口实现有界 token 状态，窗口从原成功请求完成时计起；token 绑定请求指纹、scope 和结果，并与提交协调。并发重复请求只执行一次；相同 token 不同内容返回对应 `IdempotentParameterMismatch`。避免只在成功后往 HashMap 写记录造成并发双执行。在途 token 和窗口内已提交 token 不得因容量压力被提前淘汰；容量不足时，应在新 token 对应事务发生变更前明确拒绝，已有 token 的重放仍可处理，或采用不损害完整窗口的存储策略。

**验收**：顺序重试、并发重试、客户端超时后重试均仅增量一次且 stream 一次；内容冲突拒绝，窗口过期可重用；小容量缓存写满后，首 token 在 TTL 内重放仍只执行一次，在途记录也不可淘汰。缓存容量与过期清理不得削弱幂等窗口。

### R09 — SQS DLQ 中的消息实际不可消费

**证据**：[queue/actor.rs](../crates/rustack-sqs-core/src/queue/actor.rs) `808-820` 超过 maxReceiveCount 后仅 `storage.dead_letters.push(msg)`；[storage.rs](../crates/rustack-sqs-core/src/queue/storage.rs) `21-23,64-80` 该 Vec 不计入可用/在途/延迟计数，没有目标队列转交路径，仅 purge 清空。FIFO 接收 `actor.rs:849-868` 也没应用 redrive policy。[README](../README.md) `156` 宣称支持 DLQ redrive。

**场景**：src 配置目标 dlq，maxReceiveCount=1；发送后第一次接收，visibility 设 0，再接收触发阈值。消息从 src 消失但 dlq 没收到，既未删除也未过期。

**修复契约**：queue manager / 路由 actor 负责可恢复移交，成功加入目标前不得不可逆丢弃源消息；避免两个 queue actor 相互等待造成环路。目标不存在、关闭、容量不足必须有清楚失败/重试语义；standard/FIFO 分别落实契约。

**验收**：阈值边界、移交后源不可见且目标可消费、目标关闭/删除/满时消息不丢；覆盖 FIFO，不能拿内部 Vec 长度等于 1 当 DLQ 验收。

### R10 — FIFO 复合去重键碰撞

**证据**：[actor.rs](../crates/rustack-sqs-core/src/queue/actor.rs) `499-505` 在 messageGroup scope 拼 `"{group_id}:{dedup_id}"`；[storage.rs](../crates/rustack-sqs-core/src/queue/storage.rs) `160-168` 命中便返回旧消息成功结果而不入队。

**场景**：同一 FIFO queue 开启 messageGroup 去重，发送 `(group="a:b", dedup="c")` 和 `(group="a", dedup="b:c")`；合法不同组合产生同一字符串，第二条被吞。

**修复契约**：用 `DedupKey::Queue(id)` / `DedupKey::Group { group, id }` 的结构化键，不靠换分隔符或拒绝 AWS 允许的标点。

**验收**：以上两条都可收到；同 pair 重复只一次；同 id 不同组不去重；scope 切换不和旧键空间误撞。

### R11 — Lambda 部署包错误被吞，代码更新先破坏旧文件

**证据**：[storage.rs](../crates/rustack-lambda-core/src/storage.rs) `967-987` 先覆盖 zip、删除旧 extracted；`998-1003` 只上报 `InvalidZipFile`，其他错误被忽略。`1049-1051` 无效 archive 被归为 Internal，文件创建/写入/CRC 读取错误也可走 Internal。[provider.rs](../crates/rustack-lambda-core/src/provider.rs) `611-627` 接着更新函数 metadata。

**场景**：对已有函数更新为可解码 Base64、但不是有效 ZIP 的内容，API 可能成功并登记新 hash，旧可用目录已删除。即使路径检查/大小上限返回错误，先删除旧代码仍破坏“失败更新保持原状”。现有逻辑明确为测试 stub 容忍无效 ZIP，这种便利不能进入生产路径。

**修复契约**：全部 ZIP/IO 错误传播为明确错误；先在新 staging 目录完成验证、解压、可读性检查，再原子提交代码引用。失败清理新目录而非旧目录，正常单测使用真实最小 ZIP，不保留 stub 容错分支。

**验收**：坏 ZIP、CRC 错误、写盘失败、entry 越界、解压超限后，旧 code hash / revision / 文件和可执行行为全部不变；有效更新成功切换。

### R12 — Lambda 发布版本并不拥有不可变的可执行代码

**证据**：[provider.rs](../crates/rustack-lambda-core/src/provider.rs) `949-962` 将 latest clone 成发布版本，只改版本字段，保留 code_path；更新 `611-612` 总写 `$LATEST` 目录；调用 `918-928` 从记录取 code_root，native [native.rs](../crates/rustack-lambda-core/src/executor/native.rs) `100-106` 执行其 bootstrap。另 [instance.rs](../crates/rustack-lambda-core/src/executor/instance.rs) `112-122` 的 warm key 只有 function/qualifier，不含代码 revision，更新路径没有失效旧 warm pool。

**场景**：上传 A、发布 v1、更新 latest 为 B；native 下 v1 冷启动可能执行 B，虽然它的 metadata / ZIP bytes 仍描述 A。另已预热的 latest 更新到 B 后可能仍复用 A 进程。不同 executor 取 code_root / code_zip 的方式不同，不能宣称所有 backend 都同样受影响。

**修复契约**：代码目录按不可变 digest / version artifact 管理，版本引用不能指向可变 latest 目录；warm key 纳入代码与执行配置 revision，更新/删除后旧实例不再接新工作，已在途调用按明确 drain 策略完成。

**验收**：A/B 两种明确响应的 bootstrap：v1 无论冷/热均为 A，更新后 latest 为 B；删除重建同名函数不得复用旧实例；覆盖 native，其他 backend 做版本契约回归。

## 6. 生命周期、架构与用户体验

### R13 — 停机路径不覆盖容器常用信号，也没有总期限

**证据**：[main.rs](../apps/rustack/src/main.rs) `633-635` 只等待 `ctrl_c()`；`669-670` 无限等 graceful drain；`1322-1328` serve 返回后才 save、再 providers.shutdown。[Dockerfile](../Dockerfile) `167-181` 直接以 Rustack 为入口且没有 STOPSIGNAL 改为 SIGINT。

**影响 / 限定**：普通 Unix SIGTERM 或 Docker stop 不进入应用的 Ctrl+C 保存路径；PID 1 对默认信号的表现与环境有关，可能被终止或等待至强杀，但都没有显式保存保证。Ctrl+C 下一个不完成的请求也能一直阻塞保存。后台 Event invoke / 投递不随 HTTP drain 自动结束，save-before-worker-shutdown 不等于一致切面。

**修复契约**：运行时 supervisor 独立于 snapshot registry，统一处理 SIGINT/SIGTERM，跟踪任务结果/panic、预算和取消。先停止接入与后台新工作，所有已接收工作在 quiesce 前完成或产生明确失败/取消结果，再保存、最后销毁执行资源；quiesce 不能简单实现为先清空 SQS 存储。快照只覆盖 capability 声明支持持久化的状态和副作用；未持久化消息/服务必须有摘要，不承诺重启保留。排空超时或快照失败必须清楚非成功退出，保留旧快照；不宣称 SIGKILL 能保存最新状态。

**验收**：隔离子进程中分别发送 SIGINT/SIGTERM、模拟不完成 body / 慢上游 / Event invoke；进程在配置总期限内退出。分别断言无后台变更越过一致切面、声明支持的状态恢复正确、未持久化范围有明确说明；不要求所有 202 / SendMessage 成功请求跨重启恢复。无法满足时有明确错误，旧快照仍可用。Docker stop 单独做 PID 1 回归。

### R14 — Snapshot 发布存在“目录消失但旧快照仍在”的窗口

**证据**：[snapshot.rs](../apps/rustack/src/snapshot.rs) `848-859` 先 target→backup，再 temp→target；`866-869` 仅正常错误尝试回滚；`448-451` load 发现 target 不存在直接空启动，不查可恢复 backup。

**场景**：已有成功快照 S，覆盖保存时在两次 rename 中间中断；磁盘有完整旧 `.bak`，但再次 `--snapshot S` 静默启动为空。这不是要求中断时保存最新写入，而是要求**此前成功保存的状态仍可识别/恢复**。旧文件尚可能人工恢复，不表述为一定物理销毁。

**修复契约**：选择不可变 generation + 原子 current 指针发布；保留上一有效 generation，加载验证完整性后选择 committed generation。也可采用明确可验证的 backup 恢复协议，但不能遇到可恢复旧状态仍按新名字空启动。文件/目录同步与平台替换语义必须在后续设计中明确，不能把两次 rename 称为单步原子操作。

**验收**：对每个发布步骤注入失败/终止，重启只能见完整旧或完整新状态，不能空/混合；“从未有过这个名字”和“发布中断待恢复”有不同诊断。与[Snapshot PRD](./ruststack-snapshot-prd.md)及[二进制设计](./ruststack-snapshot-binary-archive-design.md)衔接，不引入 WAL 非目标。

### R15 — 跨服务 bridge 没有完整的投递能力契约

**证据 A（实际 FIFO 错误）**：[Events provider.rs](../crates/rustack-events-core/src/provider.rs) `687-701` 保存 target 时丢弃 SqsParameters；[events_bridge.rs](../apps/rustack/src/events_bridge.rs) `55-66` 仅发 URL/body；SQS `actor.rs:479-481` 必须有 MessageGroupId。Events `849-857` 失败仅 warn。

**场景 A**：FIFO queue 开 content-based dedup，PutTargets 带 MessageGroupId=g，配置成功；PutEvents 被接受，但目标因缺 group 拒绝、队列为空。这里 PutEvents 成功只表示接受事件，本身不应被误解为同步送达承诺；真正错误是已接受的 target 参数被丢弃，且无恢复投递路径。

**证据 B（假成功）**：[main.rs](../apps/rustack/src/main.rs) `1009-1017,1033-1043` 当 runtime 禁用 SQS 时接入 Noop publisher；[SNS publisher.rs](../crates/rustack-sns-core/src/publisher.rs) `48-62`、[Events delivery.rs](../crates/rustack-events-core/src/delivery.rs) `30-38` 返回 Ok。bridge `68-74` 对未支持 target 同样 Ok。

**修复契约**：保留 app 层 bridge 隔离 core-to-core 依赖这一优点，扩展为带完整目标参数的类型化命令。runtime registry 声明依赖与支持 target；配置时拒绝不支持参数 / 目标或显式 metadata-only 状态，生产 wiring 不使用代表成功的 Noop。异步接受、投递成功、重试耗尽分别计数和可查询；不强行把 PutEvents 改成同步送达 API。

**验收**：SqsParameters roundtrip + FIFO 端到端收到 group=g；SERVICES=events 或 sns 缺 SQS 的配置能明确诊断；unsupported target 不计 delivered；队列关闭/投递失败有可观测终态。

### R16 — 配置没有统一校验和入口地址契约

**证据**：[Lambda config.rs](../crates/rustack-lambda-core/src/config.rs) `56-63` executor 解析失败静默退回 Auto/Docker，`97-107` Default 又为 Disabled；[main.rs](../apps/rustack/src/main.rs) `1280-1287` 未知或未编译服务仅 warn，若还有其他服务则启动；`256-309` CLI 静默忽略位置参数。[SQS config.rs](../crates/rustack-sqs-core/src/config.rs) `24-38` 会由 GATEWAY_LISTEN 推导端口，但 Lambda `config.rs:69-73` 仍默认 4566，其 Function URL `provider.rs:1518-1521` 使用该值。APIGW `provider.rs:86-90` 返回 AWS 域名而非本地可调用 URL。

**场景**：`LAMBDA_EXECUTOR=disable` 拼错反而选择 Auto；`SERVICES=s3,dynamdb` 启动后健康却少服务；监听 4567 时 Lambda URL 仍指 4566；直接复制 APIGW ApiEndpoint 无法指向当前本地执行入口。AWS-shaped 管理 metadata 可以保留，但必须给出明确 local endpoint 映射。

**修复契约**：一次解析为 `ValidatedRuntimeConfig`，值有来源、优先级、范围，非法安全开关/枚举/端口/服务名 fail fast；Default 与环境无值行为一致。区分 bind address 与 advertised endpoint，不信任任意请求 Host 生成绝对 URL；输出有效配置时脱敏。按 AGENTS.md § Async & Concurrency 使用统一 YAML 配置，现有 env override 的兼容与优先级显式定义，不要求本轮重写 AWS wire 格式。

**验收**：表驱动覆盖拼写错误、缺值、范围错误、重复与冲突参数；非默认端口、容器 DNS、IPv6 都生成正确且可解释的本地入口；启动前列出 compiled/enabled/disabled/degraded 及原因。

### R17 — 健康端点和探针不能证明就绪

**证据**：[gateway.rs](../apps/rustack/src/gateway.rs) `94-106` 所有注册服务固定 `running`；`84-91` 只拦 GET。[main.rs](../apps/rustack/src/main.rs) `788-804` 探针没有内部 timeout / 字节预算，读至 EOF，仅以 `200 OK` 和 `"running"` 子串判断；`1264-1268` 丢弃详细错误。[main.rs](../apps/rustack/src/main.rs) `1213-1222` CF data plane build 失败只 warn，control plane 仍注册。

**影响**：依赖/数据平面失败仍展示健康；仅运行部分服务也可被误认为全部预期服务 ready。服务不关闭连接时 CLI probe 卡住，Docker 外层 timeout 只能覆盖 Docker 用法。文本含两段关键字也不能证明 HTTP status/JSON 真健康。

**修复契约**：拆 liveness/readiness/capabilities，readiness 包括 requested services 和必要 runtime 依赖；兼容旧 health JSON，增加稳定降级原因和版本。探针用结构化 HTTP/JSON 解析、deadline、响应大小上限、stderr 诊断；HEAD 按健康接口契约处理。

**验收**：延迟恢复、缺依赖、backend unavailable、HEAD、空 services、非 200 正文夹关键字、永不 EOF、超大 body 都有确定结果；普通 probe 在预算内返回。轻量健康检查不必每次拉起 Lambda，但必须披露 backend 尚未验证/不可用，而非声称已可执行。

### R18 — 开发入口和能力说明会误导用户

**证据**：[apps/rustack/Cargo.toml](../apps/rustack/Cargo.toml) `1-14` package=`rustack-cli`、binary=`rustack`；[Makefile](../Makefile) `22-23` 却用 `cargo run -p rustack`，[README](../README.md) `311-313` 同样以旧 package 名说明 selective build。README `3,70,75-96` 混用 routed operation 数和 operation 总数，并在 `156` 宣称 DLQ redrive。快照 runtime registry 仅注册部分服务（[snapshot.rs](../apps/rustack/src/snapshot.rs) `62-111`），SQS 当前是资源型 snapshot，不代表消息持久化。旧 specs README 的多处 `rustack-*` 链接与实际 `ruststack-*` 文件名不符。

**改进契约**：区分 package/bin 名；提供可复制的首次启动、endpoint/region/credentials、一次读写、停机恢复示例。按操作列出 `implemented / partial / metadata-only / unsupported`，并明确 `routed` 不等于语义通过。snapshot 启动/保存摘要列出包含服务和资源/数据边界，不支持者必须可见；不因文档缺失强制实现全部持久化。维护单一 specs index，旧入口明确指向它。

**验收**：干净环境的文档 walkthrough 可在 5 分钟内完成启动和一次读写（不把首次编译时间计入指标）；所有示例 package 名正确；核心操作状态有对应测试或明确限制；存取 snapshot 前后用户知道哪些数据会保留。此处是待修复体验需求，不在本次修改 Makefile、CI 或现有工具链行为。

## 7. 建议的架构收敛

### 7.1 保留分层，补共享契约

不建议“大一统 service framework”或仅按行数拆 `main.rs`。应先把重复实现中已经出现的差异变成单一权威契约：

| 组件边界 | 应拥有的契约 | 关联问题 |
|---|---|---|
| ValidatedRuntimeConfig | auth mode、服务依赖、bind/advertised URL、资源预算 | R02/R05/R16 |
| Shared HTTP policy + protocol adapter | body budget、deadline、错误映射、完整性验证；协议占位符仍由各协议定义 | R03/R04/R05 |
| Runtime supervisor | 接入、后台队列、任务错误/panic、quiesce、save、终止 | R06/R13/R17 |
| Service capability registry | compiled/enabled、target 能力、snapshot 范围、readiness | R15/R17/R18 |
| Domain state owners | transaction prepare/commit、token、DLQ handoff、typed dedup key | R07–R10 |
| Immutable artifact store | ZIP staging、version artifact、warm revision、GC | R01/R11/R12 |

`SnapshotService` 不应兼任所有运行时服务的生命周期清单：没有 snapshot 能力的 Events/SNS worker 同样需要监督。内部 actor 消息以有界 channel 传递，不让队列互等；不在 DashMap guard 上跨 await。已有[SQS guard 修复规格](./fixes/sqs-longpoll-dashmap-safety.md)继续保留其不变量。

### 7.2 停机时序契约

```text
Signal / operator       Runtime supervisor         Workers / stores       Snapshot store
 │                              │                         │                     │
 │ 1 SIGINT / SIGTERM ──────────►│                         │                     │
 │                              │ 2 readiness=false       │                     │
 │                              │   stop ingress          │                     │
 │                              │ 3 stop new jobs ────────►│                     │
 │                              │                         │ finish / bounded     │
 │                              │                         │ cancel accepted work │
 │                              │ 4 quiesced ◄────────────│                     │
 │                              │   (failure: report, keep previous snapshot)   │
 │                              │ 5 export consistent state ──────────────────►│
 │                              │                         │   write generation  │
 │                              │                         │   validate + sync   │
 │                              │                         │   publish current   │
 │                              │ 6 committed / failed ◄───────────────────────│
 │                              │ 7 stop resources ──────►│                     │
 │ 8 bounded exit + status ◄─────│                         │                     │
```

整条时序共享总停机预算，各阶段只消费剩余预算。部分任务无法收拢时不把不一致快照发布成成功；保留旧版本并给出错误。具体预算值由后续运行实验校准，不把 README 的亚秒启动/保存声明当作测量结果。

### 7.3 Rust 工程约束

修复遵循 [AGENTS.md](../AGENTS.md)，不通过大范围 allow lint、忽略异常、Noop 成功实现来绕过：

- **错误**：§ Error Handling；core 使用带 source 的 `thiserror` 领域错误，app 使用 anyhow context；输入错误不能 panic，ZIP/IO 失败不可吞掉。
- **并发**：§ Async & Concurrency；actor 拥有状态、有界 channel、任务受监督且结果可观察；阻塞压缩/解压在 `spawn_blocking` 并受并发预算约束。object-safe bridge 可保留有文档理由的 async-trait。
- **类型 / 安全**：§ Type Design & API / Safety & Security；validated name、AuthMode、typed dedup key、不可变 artifact；`forbid(unsafe_code)`、checked arithmetic、secret redaction。不会为性能引入 unsafe。
- **序列化**：§ Serialization & Data；新内部配置/诊断 JSON 用 camelCase，反序列化必须走校验。**已有 AWS wire 的 PascalCase / XML 是协议要求，不应机械改成 camelCase**；这是相对通用规范的明确协议例外，保持既有服务 schema。
- **测试 / 日志 / 性能**：§ Testing / Logging & Observability / Performance；问题场景用 `test_should_*`、真实最小 ZIP、可控时钟/barrier、失败注入；tracing 仅记录 request ID、错误类别和脱敏状态，不记录 Authorization/secret/正文；优化先测量，关注峰值而非平均空闲内存。
- **文档 / API**：§ Documentation；公开错误、取消语义、边界与示例可被测试。后续涉及 Rust 的实现阶段遵循现有 Toolchain & Build 质量门禁；本 review 不新增或修改其执行规则。

## 8. 分期与验收

这是单篇 review，不另生成重复 PRD / design / roadmap 文档。下列 milestone 面向用户结果，工程阶段与其 1:1 配对；具体服务设计仍为协议权威。

### 8.1 用户可见里程碑

| 里程碑 | 用户获得什么 | 退出条件 |
|---|---|---|
| M0 安全边界可信 | 错误凭证不静默放行，文件/正文/上游目的地不可越界 | R01–R04 负向场景通过，危险默认部署组合有明确阻断/选择 |
| M1 有界运行与可靠退出 | 慢请求、并发函数、容器停机不会不可控 | R05/R06/R13/R14 场景通过，旧快照可恢复 |
| M2 数据与版本可信 | 事务重试、DLQ、FIFO 和 Lambda 版本语义可依赖 | R07–R12 全部达到各自不变量 |
| M3 集成可预测 | FIFO target 参数保留、依赖缺失/错误配置能早发现 | R15/R16 端到端与错误配置场景通过 |
| M4 首次使用与诊断可信 | 开发入口能复制运行，健康/能力/持久化说明诚实 | R17/R18 walkthrough 与故障诊断通过 |

### 8.2 工程依赖顺序与粗估

| 阶段 | 先做的契约，再做的消费者 | 交付与验证 | 粗估人日 |
|---|---|---|---|
| Phase 0 → M0 | 先补失败回归与威胁模型；validated name / AuthMode / payload policy / egress policy，再接全部入口 | R01–R04；补服务差异清单，不把仅一个 HTTP crate 修复当全系统完成 | 6–10 |
| Phase 1 → M1 | 先运行时预算和 supervisor，再 Event queue / body adapter / stop / snapshot publish | R05/R06/R13/R14；必须包含真实子进程与发布步骤故障注入 | 8–13 |
| Phase 2 → M2 | 先 DDB commit coordinator 再 token；先 DLQ handoff 再用户可见计数；先 immutable artifact 再 warm revision | R07–R12；数据、stream、代码行为联合断言 | 12–20 |
| Phase 3 → M3 | 先 target/capability/config schema，再 bridge 和配置输出 | R15/R16；standard/FIFO、不同 SERVICES/端口组合 | 5–8 |
| Phase 4 → M4 | 用真实 registry 生成 readiness/能力说明，再修使用示例与导航 | R17/R18；干净环境 walkthrough、文档链接检查 | 3–5 |

合计 **34–56 人日**，按单人全职约 **7–12 工作周**，不含新服务功能。此为基于源码改动面、并发与故障验证成本的规划区间，**不是 benchmark 或研究 memo 验证过的工期**；Phase 0 完成后重估。SQS、DDB、Lambda 的领域实现可并行，但共享 HTTP/supervisor 契约先落地，避免消费者反复返工。

优先级与阶段不同：M0 后尚不能把服务当成多租户平台；M1 的监督与预算先于重投递等新后台逻辑，否则越修功能越放大资源问题。每阶段必须通过其关联 R 项测试，不能只以“接口存在 / 返回 200 / 文档已写”退出。

### 8.3 验证边界与待覆盖面

- 所有负向安全实验限定 loopback、临时目录和自建上游，不访问真实 metadata 服务或真实账号。
- 事务/去重使用固定输入、可控时间和并发 barrier；DLQ/版本/bridge 使用实际状态所有者，而非返回成功的 mock。
- 资源测试同时量化 admission、实际字节、进程/任务/FD 峰值和取消释放；保留吞吐/延迟基线，防过度限流破坏正常长轮询和大对象上传。
- 延续研究中 SDK + compatibility suite 分层策略：DynamoDB Alternator、S3 Mint、SQS 有针对性的多 SDK 测试；不能用 happy-path CRUD 覆盖数证明完整兼容。
- 全量 RustSec/许可证审计、TLS 配置、IAM/KMS 密码学、所有分页/表达式边界、S3 对象生命周期、CloudFront cache correctness、各 feature 组合仍需后续专项审查；本次不对未深查区域出具“无漏洞”结论。

## 9. 关键决策与已排除误报

### D1 — 按模拟器边界评估，而非要求生产 AWS

保留 metadata-only、localhost fixture、可信 native 的合理用途；选择显式能力和 operator policy，而不是默认宣称全功能/安全多租户，也不一刀切禁止所有本地 origin。关联 R04/R15/R18。

### D2 — 边界和原子性修在状态所有者，不补表面 if

R01 用受控 artifact 路径，R02 用可表达合法状态的 AuthMode，R07 用原子提交，R10 用结构化键。仅增加一条 regex、换分隔符、加一次预检或换成 DashMap 不能替代这些契约。

### D3 — runtime lifecycle 不依附持久化支持

选择独立 supervisor + capability registry；SnapshotService 只负责可持久化状态。未持久化的后台任务也必须被停机管理。关联 R06/R13/R17。

### D4 — 先完整生成新状态，再提交可见引用

ZIP artifact 和 snapshot generation 都采取 stage/validate/commit；反对就地覆盖后尝试回滚。此决定不要求为普通业务写入增加 WAL。关联 R11/R12/R14。

### 已排除 / 降级

- Lambda ZIP entry 已有 `enclosed_name`、250 MiB 声明/实际解压字节限制，不能报成“完全没有 Zip Slip / zip-bomb 防护”；R01 针对外层 FunctionName，R11 针对错误与提交顺序。
- snapshot zstd 解压使用 `take(MAX_ARCHIVE_BYTES + 1)`（[archive.rs](../apps/rustack/src/snapshot/archive.rs) `518-530`），不能报无界解压；多任务总预算可继续改进。
- SNS 当前非 SQS 协议分支不执行 HTTP 请求；endpoint 存储宽松不是已可达 SSRF。CloudFront 的 in-process S3 origin 同样不是任意网络 fetch。
- native 的直接执行是有意能力，不称作 shell 注入或 VM 逃逸；其默认部署组合仍须明确告警/安全模式。
- SQS 资源型 snapshot 不保消息、未纳入 registry 的服务不持久化，是阶段性能力边界；本次归 R18 的披露要求，不要求无条件实现全服务持久化。
- 不把静态常量构造 response 的 `expect` 与外部输入可达 panic 混为一谈，也不把每个 Mutex 一律判为死锁。
- 当前 [Cargo.lock](../Cargo.lock) `5855-5856` 是 zip 8.5.1；查阅 [RUSTSEC-2025-0168](https://rustsec.org/advisories/RUSTSEC-2025-0168.html) 的受影响范围是 `>=1.3.0, <2.3.0`，**不适用于该锁定版本**。R01 是应用路径逻辑缺陷，不借该公告冒称依赖 CVE。没有运行全量 audit，不能推出其他依赖无风险。

## 10. 交叉引用与外部校对

### 既有设计

- [Lambda executor](./ruststack-lambda-executor-design.md)、[Squib runtime](./ruststack-lambda-squib-runtime-design.md)、[S3 code packages](./ruststack-lambda-s3-code-design.md)：代码包与执行约束。
- [DynamoDB](./ruststack-dynamodb-design.md)、[操作差距计划](./service-operations-gap-impl-plan.md)：事务、token 语义。
- [SQS](./ruststack-sqs-design.md)、[EventBridge](./ruststack-events-design.md)、[SNS](./ruststack-sns-design.md)：队列和投递协议。
- [Snapshot PRD](./ruststack-snapshot-prd.md)、[二进制归档](./ruststack-snapshot-binary-archive-design.md)、[验证计划](./ruststack-snapshot-verification-plan.md)：保存边界与恢复。
- [CloudFront data plane](./rustack-cloudfront-dataplane-design.md)、[APIGW](./ruststack-apigatewayv2-design.md)：代理集成。

### 研究背景（历史结论，不作为当前版本实测）

- [Cargo Lambda 执行 spike](../docs/research/spike-cargo-lambda-runtime-execution.md)：bootstrap/Runtime API 合约与 macOS/Linux artifact 区分；本次不重新否定已验证的打包路径。
- [DynamoDB API](../docs/research/dynamodb-api-research.md)、[测试分层](../docs/research/dynamodb-integration-test-suites-research.md)：原子性/隔离需求与 SDK/Alternator 分层。
- [SQS API](../docs/research/sqs-api-research.md)、[SQS suites](../docs/research/sqs-test-suites.md)：FIFO/时序及协议覆盖要求。
- [LocalStack 容器/CI](../docs/research/localstack-container-ci-research.md)：统一入口、健康、信号/容器集成背景。
- [LocalStack S3](../docs/research/localstack-s3-research.md)、[S3 suites](../docs/research/s3-integration-test-suites-research.md)、[s3s](../docs/research/s3s-crate-research.md)、[Smithy server codegen](../docs/research/smithy-rs-server-codegen-research.md)、[SSM](../docs/research/ssm-parameter-store-research.md)：分层与能力范围背景；不据其历史版本表建议盲目迁移依赖。

### 外部资料（2026-09-07 查询）

- [http-body-util Limited API](https://docs.rs/http-body-util/latest/http_body_util/struct.Limited.html)：实际 poll 字节上限，可用于 R05；查询时 docs.rs 最新为 0.1.5，方案不要求本次升级依赖。
- [reqwest redirect policy](https://docs.rs/reqwest/latest/reqwest/redirect/index.html)：默认自动跟随最多 10 跳，支持 R04 的默认客户端行为判断。
- [RustSec zip advisory](https://rustsec.org/advisories/RUSTSEC-2025-0168.html)：仅用来排除错误 CVE 归因，不代替全量依赖审计。

后续修复应按 R-ID 关联实现、回归测试和能力说明；关闭条目时记录实测证据与范围，而不抹去这份审查基线。
