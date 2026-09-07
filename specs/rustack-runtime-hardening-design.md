# Runtime hardening：配置、准入、停机与快照

状态：实施契约。依赖：[review](./rustack-system-review.md) R05/R13/R14/R16/R17/R18、[实施计划](./rustack-system-hardening-impl-plan.md)。不替代领域状态的原子性，也不扩展 snapshot 数据范围。

## 1. 一次解析配置

`RUSTACK_CONFIG` 指定 YAML 文件；未指定时不要求文件存在。配置 schema：`environment` 为已有环境配置名称到标量值的映射（兼容现有服务变量命名），`budgets` 为 camelCase 的强类型资源预算，`advertisedEndpoint` 为可选本地公共入口。顶层未知字段拒绝、environment未知键拒绝，数组/嵌套对象不允许伪装 scalar；值先转换为字符串后执行同一范围/枚举/布尔/字节校验。真实进程环境覆盖 YAML，默认值最后。不能通过 `set_var` 注入：使用一次安装的已验证 settings facade，各 service config 的 `env::var` 改经该 facade，测试未初始化时仍能独立构造配置。

读配置使用 config crate 的 YAML source（显式feature），错误含 key 和来源、不得含 secret 值。`--help`/`--version` 不触发文件/网络；运行和 `--health-check` 使用同一有效配置。位置参数、未知服务名、重复/冲突 flag 拒绝。任何 strict signature=false skip 配置若缺配对credentials，在监听前失败；协议层仍独立 fail closed。

bind 默认 `127.0.0.1:4566`；advertised endpoint 默认由bind推导（wildcard变loopback），不能从请求Host信任推导，IPv6必须括号包围。Lambda等URL由有效endpoint生成；APIGW可保留AWS-shaped ApiEndpoint，但capabilities明确提供 `/_aws/execute-api/{api}/{stage}` 本地映射。

## 2. Gateway / supervisor

`RuntimeStatus` 共享原子 serving/draining/failed 状态，capability records列出service、compiled/enabled、snapshot kind、执行与target能力；liveness证明进程能响应，readiness证明请求服务已安装且未draining/failed。配置了不可用的必要backend应fail startup或明确degraded，不把未执行runtime probe描述成已验证。

每连接获取permit（默认256）；每业务请求获取独立permit（128），permit保留到响应body完成/丢弃，不在拿到response headers后提前释放。health无需业务permit，但连接也有总限额；HTTP/2业务streams仍受请求permit覆盖。HTTP读头5秒，控制面总请求30秒；S3大对象由协议body idle/实际字节预算约束。错误在原body消费前也不能触发无界drain。超额503/协议错误附request id。

supervisor用JoinSet跟踪connection任务（panic/IO错误有日志和诊断）。SIGINT/SIGTERM共用停机路径；先标draining、停止accept，已有connection graceful drain在剩余总预算内完成。runtime worker registry独立于SnapshotService，包括Lambda/Events/SNS/SQS，按Events/SNS→Lambda→SQS顺序quiesce，禁止新的cross-service投递越过切面。失败/超时中止保存并保留旧快照；最终shutdown销毁worker，不重启后回报假成功。正常save必须发生在quiesce后；unsupported persistence只有摘要，不能承诺全部消息恢复。

```text
┌─ immutable validated config ──┐
│ YAML < env; budgets; endpoint │
└─────────────┬────────────────┘
              ▼
┌─ Gateway ───────────────────────────────────────────────────────┐
│ connection permits → request permits → protocol IO budgets       │
│ health/status ◄─ RuntimeStatus                                   │
└─────────────┬────────────────────────────────────────────────────┘
              ▼
┌─ Supervisor (independent of snapshots) ──────────────────────────┐
│ JoinSet connections + typed worker handles                       │
│ SIGINT/TERM → drain → Events/SNS → Lambda → SQS quiesce            │
│                      failure ──► no save, report nonzero exit     │
│                      success ──► snapshot.save → workers.stop     │
└──────────────────────────────────────────┬───────────────────────┘
                                           ▼
                                  previous / next snapshot
```

## 3. 快照恢复协议

保留当前 `<name>/manifest.ss.zst` 用户目录及旧文件格式。覆盖写使用稳定 `<name>.previous` recovery目录（内部隐藏名），受同名snapshot排他锁保护；写temp全部shards/manifest并sync，再target→previous，temp→target，再sync父目录，最后清理previous。发布过程中任何失败保留/恢复完整旧目录，不忽略restore失败。load前在锁内执行恢复：target缺失且previous存在则验证previous manifest与完整性并恢复；target存在且previous存在则验证target（失败回退previous），绝不静默空启动。没有target/previous才是初次空状态。并发两个Rustack写同名snapshot必须拒绝，不last-writer-wins，锁必须跨进程且异常退出能释放。输入快照路径symlink/components继续按archive既有边界验证。

测试注入target→previous之后失败、temp→target前/后失败、恢复失败、同时load/save、损坏目标/完好previous；恢复只能完整旧/新。该协议不是WAL，不保证SIGKILL前的最新请求被持久化，但必须保住旧已完成快照。

## 4. 探针和用户诊断

`/_health/live`是liveness，`/_health/ready`是readiness；兼容 `/_localstack/health`、`/_health`、`/health`与MinIO路径，支持GET/HEAD。capabilities用独立JSON端点，提供version、services状态、snapshot coverage、execution restrictions/local URL patterns。HEAD无body，content type/status与GET一致。probe解析真实HTTP status和JSON readiness，3秒总deadline、64KiB上限、不用正文substring；stderr包含connect/status/parse/timeout类别且不含secret。

新用户文档以package rustack-cli、binary rustack区分；5分钟walkthrough不含首次编译。能力表不使用routed数量代替实现，部分metadata-only明确列出。配置示例YAML+env优先级、loopback容器publish、native显式可信选择、快照资源/数据覆盖都可复制。

## 5. 规范与验收

Errors、Async、Safety、Serde、Testing、Tracing、Performance与Documentation按AGENTS.md对应章节。新边界禁止unwrap/expect/index panic/unsafe。public类型文档与脱敏Debug齐全，使用有界channel/JoinSet和非零预算。回归覆盖配置坏值、CLI冲突、IPv6、伪HTTP health、慢/超大响应、SIGINT/TERM子进程、deadline、连接/请求permit释放与snapshot故障恢复。后续执行记录附在实施计划，不在实现尚未验证时标implemented。
