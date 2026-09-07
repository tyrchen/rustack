# 运行时硬化操作与能力指南

本文面向把 Rustack 当作本地 AWS 替代品的开发与 CI 操作者，记录 R01–R18 修复后**可观察的安全边界、执行与持久化范围、配置格式**，以及从旧默认值迁移的影响。协议层面的详细理由见各硬化设计 spec。

## 1. 配置来源与验证

### 1.1 三层来源

启动时按以下优先级合成一份**不可变**设置（`rustack-core::settings`）：

1. YAML 文件（`RUSTACK_CONFIG` 指向的路径，≤1 MiB，5 秒读取截止）；
2. 进程环境变量（覆盖 YAML 中同名 `environment` 键）；
3. 编译期默认值（最后应用）。

YAML 顶层结构：

```yaml
# 配置示例：显示所有可用键；按需保留
environment:
  GATEWAY_LISTEN: "127.0.0.1:4566"
  SERVICES: "s3,dynamodb,sqs,lambda"
  LOG_LEVEL: "info"
  LAMBDA_EXECUTOR: "disabled"        # 执行器必须显式选择
  S3_SKIP_SIGNATURE_VALIDATION: true # 本地开发免签名；false 需要凭据
  ACCESS_KEY: "local-test"
  SECRET_KEY: "local-secret"
budgets:
  connections: 256                    # 最大 HTTP 连接（含少量诊断保留位）
  requests: 128                       # 同时在飞业务请求
  headerSeconds: 5
  requestSeconds: 30                  # 控制面请求总截止
  lambdaInvokeSeconds: 930            # 同步 Invoke 总截止（可为 900 s 执行 + init）
  shutdownSeconds: 30
  controlBodyBytes: 16777216
  lambdaCodeBodyBytes: 100663296
  upstreamBodyBytes: 67108864
  bodyIdleSeconds: 5
  bodyTotalSeconds: 30
  s3BodyTotalSeconds: 3600
  s3ObjectBodyBytes: 5368709120
advertisedEndpoint: "http://localhost:4566"
```

### 1.2 验证规则（失败即拒绝启动）

- YAML 未知键、非标量、坏数值、越界预算一律拒绝，错误不打印配置值；
- 布尔键只接受 `true/false/1/0/yes/no/on/off`，执行器枚举只接受 `disabled/native/auto/docker/squib`；
- `DEFAULT_REGION` 与 `AWS_DEFAULT_REGION`、`ACCESS_KEY` 与 `AWS_ACCESS_KEY_ID`、账号别名之间冲突拒绝；
- `GATEWAY_LISTEN` 只接受带非零端口的 IP（含 IPv6），裸进程默认 `127.0.0.1:4566`；
- 已启用且声明 `_SKIP_SIGNATURE_VALIDATION=false` 的服务，若缺少完整 access/secret 凭据则拒绝启动（**凭据缺失不再放开认证**）；
- `SERVICES` 里出现未编译或未知服务名直接报错退出，不再静默跳过；
- 首个参数不是已知旗标时按未知参数报错；`--health-check` 与 `--snapshot` 互斥且各自只能出现一次。

`--help` / `--version` 不需要读取或验证配置，保证廉价无副作用。

### 1.3 从旧默认值的迁移影响

| 项目 | 旧行为 | 新行为 |
|---|---|---|
| 裸进程监听 | `0.0.0.0:4566`（所有接口） | `127.0.0.1:4566`（回环） |
| 容器镜像监听 | 无声明 | `0.0.0.0:4566`（Dockerfile 显式设置） |
| Lambda 执行器 | 未设置回退 docker/native | 显式 `disabled`，Invoke 明确报错 |
| 无凭据 + 严格签名 | 放行 | 启动失败（fail closed） |
| 未知/未编译服务 | 警告并跳过 | 启动失败 |
| 冗余 CLI 参数 | 静默忽略 | 启动失败 |

## 2. 信任边界与请求处理

- 每条业务请求在网关获得**在飞许可**（`budgets.requests`），许可持有到**响应体被消费完**而不是只到响应头；健康/诊断请求走保留连接且不占业务许可。
- 每个 DATA 帧在实际复制前接受字节/总时长/空闲时长三重检查（16 MiB 控制面、96 MiB Lambda ZIP JSON、64 MiB 上游、S3 对象 5 GiB 流式、响应按 S3 流式预算）。
- 签名校验使用**实际收到的字节摘要**，不接受请求声明的 `x-amz-content-sha256` 替身；`UNSIGNED-PAYLOAD` 及流式签名标记只对 S3 有收窄豁免；未知流式格式拒绝。日志与错误不输出签名材料。
- 上游/集成客户端：不跟随任何重定向、不继承 `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`；请求同时受 idle 与**总** wall-clock 截止约束（APIGW 遗留记录无 timeout 时默认 30 s 兜底）；`advertisedEndpoint`/集成 URL 之外的本地回环目标只有在显式配置的模拟器拓扑（本机 SDK/CLI 或本机 fixture 源）中允许。
- **边界**：请求侧的读 idle/total deadline 在响应**发送**期间不重启；若客户端停止读取响应，hyper 停 poll 后相关 permit/连接会保留到停机 drain（默认 30 s）才释放。面向停滞客户端的独立写 deadline 未实现，属已知边界而非不受限保证。
- Lambda FunctionName/Qualifier/ARN 在 resolver、store、快照导入各边界独立校验（1–64 ASCII 标识符），逻辑名永不拼接进文件系统路径。

## 3. 执行、投递与持久化范围

### 3.1 Lambda

默认 `disabled`；Docker **不支持并显式报错**。`native`/`auto`（squib 需要可用虚拟化）是**可信主机执行**，不是沙箱。执行准入：全局 32、每函数默认 8，保留并发为 0 时拒绝；异步事件预算 128（采用当前 AWS 异步载荷上限 1 MiB，同步 6 MiB）。ZIP 上传不可变、按内部 UUID 落地，`latest` 与已发布版本不再共享同一可覆盖路径；坏 ZIP/CRC/路径/大小/IO 在发布前被拒绝且保留先前版本与 warm 指纹。

### 3.2 快照（`--snapshot <name>`）

- 目录 `RUSTACK_SNAPSHOT_DIR`（默认 `.rustack/snapshots`）下按名字排他锁（`.{name}.lock`），同一快照名同时只能一个运行时拥有；
- 保存：staged 目录写入与 fsync → 校验 manifest → 发布为目录级原子替换，替换前保留 `.{name}.previous`，崩溃或发布间隙通过恢复逻辑回到上一个完整代际；发布失败/恢复失败会**报错退出**，不会静默以空状态继续；
- 加载：先恢复再读取；目标缺失且无可恢复代际才视为空起始（首次运行建快照的合法场景）；
- **覆盖范围不等于全量持久化**：S3/DynamoDB 持久资源+数据，DynamoDB Streams 记录，Lambda 元数据+代码（不可变 artifact），其余服务为资源或缓存；SQS **消息**、Events/SNS 投递历史、无快照声明的服务不随快照保留。日志明确打印本次 coverage。

### 3.3 停机（SIGINT/SIGTERM）

收到信号后：停止接受连接（保留诊断连接）→ 在共享总 deadline（`budgets.shutdownSeconds`，默认 30 s）内排空 HTTP → 按顺序 quiesce：Events/SNS 停新投递并等待已接受投递 → DynamoDB 排空已准入 blocking 工作 → Lambda 停新执行并收割子进程 → SQS 停新移交并排空 DLQ 移交 → 快照保存（有快照名且前序成功）→ 并行停止各 provider。quiesce/保存超时不发布一致性快照；总截止前留出清理余量，被卡住的 blocking IO 不会让进程无限挂起。

## 4. 健康与能力端点

GET/HEAD（不占业务许可、无签名要求，仅供运维探针）：

| 路径 | 语义 |
|---|---|
| `/_localstack/health`、`/health`、`/_health` | 兼容别名，与就绪相同响应体 |
| `/_health/ready`、`/minio/health/ready` | **就绪**：网关未排空、worker 存活、服务非空且每个状态 `running` |
| `/_health/live`、`/minio/health/live` | 存活（进程在）；排空时仍 200 |
| `/_rustack/capabilities` | 结构化能力：逐服务 `status/snapshot/execution`、`ready`、worker 投递计数、本地模拟端点说明 |

`services` 映射中受管 worker（Events/SNS/SQS/DDB）失败终态标 `failed`；未声明执行后端时（如 Lambda 默认 `disabled`）标 `disabled`（是明确能力而非故障）；排空阶段标 `draining`。`rustack --health-check` 与 readiness 探针接受 `running` 与 `disabled`，拒绝 `failed`/`draining`。`workers` 字段含 `delivery.{accepted,delivered,failed,rejected}` 计数（投递口径，API 接受不等于投递成功）。

`rustack --health-check` 现在走真实 HTTP 状态码 + 有界响应解析（≤64 KiB、3 秒总截止、解析 JSON `ready`/服务状态），不再字符串匹配 `200 OK`/`"running"` 子串。

## 5. 客户端建议与示例

```bash
# 仅本机使用：默认即安全，无需额外设置
rustack
export AWS_ENDPOINT_URL=http://127.0.0.1:4566 AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test
aws s3 mb s3://my-bucket

# CI 健康检查
rustack --health-check

# 严格签名（可选，需要凭据）
RUSTACK_CONFIG=/path/to/rustack.yaml SERVICES=s3,dynamodb \
AWS_ACCESS_KEY_ID=my-access AWS_SECRET_ACCESS_KEY=my-secret rustack
```

建议显式 `SERVICES=` 列出 CI 实际使用的服务：启动更快、能力面更小、误配置在启动即报错。

## 6. 常见失败与诊断

| 症状 | 原因 |
|---|---|
| 启动即退出，提示 invalid configuration | YAML/环境值或别名冲突；见 1.2 |
| Invoke 返回 Lambda 不可用 | 默认 Disabled；需显式 `LAMBDA_EXECUTOR=native`（可信执行） |
| 跨容器/远程 SDK 连不上 | 容器显式 `GATEWAY_LISTEN=0.0.0.0`；裸进程默认回环 |
| SQS/Events 消息在重启后消失 | 资源型快照不保留消息；quiesce 只排空 DLQ 移交不承诺持久化 |
| `--health-check` 报告 unhealthy | 用 `curl /_health/ready` 与 `/_rustack/capabilities` 查 worker/投递计数 |
| 快照目录出现 `.name.previous` | 正常发布保留最近一次提交用于崩溃恢复；无歧义时由下次启动清理 |
