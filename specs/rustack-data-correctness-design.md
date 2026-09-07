# 数据正确性与跨服务投递设计

状态：实施契约。范围：系统审查 R07/R08/R09/R10/R15；实施任务 M2.1、M2.2、M3.1。

## 1. 权威与边界

本设计细化 [系统审查](./rustack-system-review.md) 和 [实施计划](./rustack-system-hardening-impl-plan.md)。协议依据 [DDB](./ruststack-dynamodb-design.md#116-transaction-operations)、[SQS](./ruststack-sqs-design.md)、[Events](./ruststack-events-design.md)、[SNS](./ruststack-sns-design.md)。研究依据 [DDB API](../docs/research/dynamodb-api-research.md#35-transaction-operations-2)、[SQS API](../docs/research/sqs-api-research.md#74-dead-letter-queues-dlq)。旧设计中的 Noop 成功、unsupported 只日志策略由本设计明确替代。没有新增依赖，不实现网络 SNS、WAL、跨进程 exactly-once 或消息快照持久化。

## 2. DDB：协调、prepare、commit

### 2.1 所有操作的同一隔离域

现有 provider 为同步 API，HTTP handler 同步调用，且 snapshot 也是同步入口。保留 API，采用 provider 唯一 `parking_lot::Mutex<()>` 操作门（已有依赖），只协调临界区，不将数据、map、非 Send 状态包入锁。状态继续由已有 service/table storage 持有。所有公开 handle 操作（普通读写、batch、Query/Scan、元数据 CRUD 和 transaction）先取得操作门；内部组合调用使用私有 `*_inner`，禁止递归获取门。reset/import/export 同样协调。不存在 transaction-only 锁。

选择理由：将这些同步 API 强行转换为 Tokio actor 会使同步调用在当前线程 runtime 阻塞/死锁，或要求全 HTTP/snapshot API 异步迁移，超出本 slice。单位锁是同步临界区协议，不是 `Mutex<HashMap>`，没有锁跨 await；串行化全部本地 DDB 操作换取可审查的串行化隔离。未来 actor 迁移可沿相同命令边界完成，不改变本不变量。HTTP handler 在 spawn_blocking **之前**从 provider 共享的128槽 semaphore取得 RAII permit，permit移入blocking closure（HTTP future取消不释放它），操作结束才释放并notify。`RustackDynamoDB::quiesce().await -> Result<(), DynamoDBError>`先close admission、排空permit，再通过blocking barrier等待已执行的同步provider操作。普通handle在操作门内检查admission关闭，排队未开始的操作显式失败；snapshot/reset/import是父控制的管理面，仍允许在quiesce后持门执行。父在HTTP关闭后Events/SNS之后、snapshot之前调用DDB quiesce，外包总deadline；quiesce取消不重新开放准入，可重试。新provider构造即restart，不复用已停机实例；不修改父拥有的gateway。

```text
普通 CRUD / batch / Query / Scan / transaction / snapshot
                         │
                         ▼
              ┌─ 唯一 provider 操作门 ─────────────────────┐
              │ token lookup/admission                     │
              │ validate keys / all expressions/conditions │
              │ prepare Vec<validated mutation>            │
              │ failure ──► discard (no data/stream)        │
              │ success ──► infallible storage commit       │
              │          ──► publish stream                │
              │          ──► token completed(now)          │
              └────────────────────────────────────────────┘
```

### 2.2 Prepare 产物和不可失败提交

先检查1..100 action、每项恰好一种 action、表存在、重复(table,key)，合法 key 类型/非空/长度及 key-only 请求无多余属性。逐项解析并计算条件和 UpdateExpression，验证名称/值占位符、路径冲突、不可修改主键、数值/空集合/400KiB item/4MiB transaction 限额。prepare 只生成已计算最终 item、已解析主键、旧 image 与 table Arc，不修改 storage、不发 stream。ConditionCheck 同样在协调域里求值。

提交 storage API 接收已验证主键，不重新解析、不返回可恢复错误。所有 mutation 完成后才发布 stream。BatchWriteItem 同样先构建已验证 mutation 集合，共用不可失败提交与提交后 stream 路径，避免 malformed 后项让整个请求报错却已写入前项；这不承诺 AWS 批操作本身具有跨请求事务属性。删除不存在 item 不发布 Remove；更新/put 发布一条 Insert/Modify。失败 prepare 无部分数据、计数或 stream；普通条件写也在整个检查与写入期间持门。TransactGetItems 整体持门，故不可能看到两次提交混合。snapshot 导出在门内获得一致切面；导入应先构建合法替换状态再发布。

### 2.3 Token 状态

scope 为 provider 的 account/region。token 1..36 bytes，指纹是去掉 token 后请求的稳定 JSON（递归排序对象键）的 SHA-256，复用已有 rustack-auth::hash_payload，不新增依赖；覆盖所有事务内容与返回选项，避免随机 HashMap 顺序或弱哈希碰撞。最多1024记录，以 DashMap 存放，操作门下读写；每条仅保存64字节十六进制摘要、完成时间和响应，不保留请求正文。指纹序列化上限8MiB，另有每事务4MiB/100操作预算。可测试配置小容量/受控时间。

门的排他性使进入 prepare 的事务唯一：同 token 并发请求等待门，第一条失败不记录；成功后第二条重放已存结果而不求值条件/不写/不发 stream。容量准入在任何 mutation 前；只删除完成时间距今不少于600秒的成功记录；不能驱逐未过期成功记录。当前实现无独立在途条目：门内的新 token 已保留一个容量槽，门外请求不能进入或淘汰它，因此等价在途保护。窗口从 commit+stream 完成计时，重试不延长窗口；冲突返回 IdempotentParameterMismatchException；满时新 token 返回明确限流错误，已有 token 仍重放；没有 token 的事务不占token容量。reset/import 清除token，token不承诺跨重启幂等。

## 3. SQS DLQ：路由经理与actor异步移交

保留每队列actor唯一消息状态所有权，队列 registry 持有可克隆 routing handle。源actor不等待目标actor：通过manager路由的有界 `try_send` 将 Transfer 命令送目标，并保留消息及 oneshot acknowledgment 到 pending handoff；正常tick轮询ack。目标actor同步校验/入队/回复，其后source删去保留副本。禁止 actor A await B 或为每条消息spawn任务。

```text
source available ── receive_count >= max ──► reserve pending (FIFO保持group blocked)
       ▲                                      │ manager resolve exact ARN
       │ target missing/closed/full           ▼
       └──── retryable restore ◄── negative ack / try_send failure
                                              │ bounded Transfer command
                                              ▼
                                     target actor validate/enqueue
                                              │ success acknowledgment
                                              ▼
                                    source release reservation/group
```

源仅在目标明确成功后忘记消息。关闭/找不到/队列类型不同/消息超过目标限制/目标命令容量满时恢复源可见并保留receive_count，下一次receive重试，不能把失败消息交给普通consumer绕过redrive。不用不可消费dead_letters Vec。pending有固定上限128，每次receive最多10；满时不移走消息。pending纳入不可见计数；FIFO组在成功/失败恢复前保持阻塞，失败恢复到组头。目标保留body、attributes、group及source ARN；重置receive_count/first_receive_timestamp，FIFO dedup用源消息ID防正常dedup误吞，standard保留sent timestamp，FIFO迁移重设sent timestamp。移交的enqueue不再应用源阈值。

目标 Transfer 不执行网络IO；ack无超时重放歧义：通道关闭说明命令未确认，只有目标未提交才允许恢复。实现必须保证目标提交与成功ack的相邻同步步骤，源actor在pending存在时不提前停止。runtime quiesce停止新hand-off并排空已有pending，保存后才shutdown队列；不能用shutdown_all作为quiesce（它清空消息）。显式Delete/Purge仍是用户要求删除数据，不承诺保留被删除队列的消息。消息仍不随资源型snapshot持久化。

## 4. FIFO去重键

`DedupKey::Queue(String)` 与 `DedupKey::Group { group: String, id: String }` derive Eq/Hash。禁止字符串拼接。两个scope属于不同键空间，切换scope不会命中旧scope的entry。合法标点不收紧；(a:b,c)、(a,b:c)都入队，同pair仅一次，同id不同group不互相去重。

## 5. Events/SNS：完整参数和能力

### 5.1 配置与wire

Events直接存完整 `Target`，ListTargetsByRule clone原模型，不重建丢字段。SqsParameters为显式结构，AWS目前字段 `MessageGroupId`（1..128 ASCII可打印非空字节，保留标点），未知字段拒绝，不把JSON Value无校验传下游。TargetDelivery接收完整Target及已转换body，且提供同步validate方法。仅SQS target可执行；unsupported ARN/参数明确失败PutTargets的该entry。暂不执行RoleArn、retry_policy、dead_letter_config及非SQS参数，拒绝而不是暗示受支持。Input/InputPath/InputTransformer仍按现有实现，但互斥且限制长度/数量。FIFO target必须有MessageGroupId；bridge真实SendMessage携带该值，使用SQS content-based dedup，若目标未启用则得到明确delivery failure（不得凭空生成AWS不支持的SqsParameters字段）。

UnavailableTargetDelivery/UnavailableSqsPublisher替代生产Noop，validate与deliver都返回Unavailable。应用registry声明Events/SNS仅有SQS投递能力；未启用SQS时资源CRUD可工作，但配置SQS target/subscription返回显式错误，readiness显示degraded。core不依赖其他service core，bridge仍在app中。

SNS仅支持SQS订阅执行（每topic最多128订阅）；不支持的protocol在Subscribe拒绝；SMS/platform publish明确unsupported（对应CRUD metadata可保留）。SQS publisher同步validate精确ARN/scope；FIFO topic→FIFO queue保留group/dedup，FIFO topic→standard queue不传FIFO-only参数，standard topic→FIFO queue明确unsupported。SNS envelope保留属性；raw delivery当前仅支持无message attributes，带属性的raw publish明确拒绝（不能默默丢属性）。RedrivePolicy/DeliveryPolicy/SubscriptionRoleArn在Subscribe和SetSubscriptionAttributes均明确拒绝。

### 5.2 有界接受、终态、停机

Events保持PutEvents异步接受语义，每event匹配形成有界delivery batch，最多128目标，每事件body上限256KiB；入有界128槽actor channel成功才返回event_id，满/停机/无runtime该entry返回明确错误且不发生任何该事件投递。单worker按入队顺序处理，target每次投递5秒deadline，失败不重试（最多一次尝试、明确终态；配置retry被拒绝）。单event目标顺序确定，FIFO同目标事件不会因无界spawn乱序。worker用受控子任务执行一次调用并await，panic/timeout/失败分别记录为failed，不杀死worker。

公开delivery_stats显示accepted/delivered/failed/rejected，pending=accepted-delivered-failed（delivery attempt口径另明确）；不记录body/secret。quiesce关闭新接受、让已接受工作完成并等待worker确认，一致切面后没有后台变更。shutdown幂等、同样等待结束；父runtime外包总剩余timeout，超时必须非成功且不保存一致快照。构造器保持同步，可在无runtime做metadata操作；worker首次提交lazy启动，无runtime拒绝而不是panic。

SNS当前publish在请求内await fanout，保留同步执行而非人为改异步承诺；在开始任何delivery前获取有界128个publish permit，每次下游调用5秒timeout。quiesce关闭permit准入并等待全部已接受publish结束，shutdown同义，统计成功/失败明确可查询。无每请求detached spawn。失败delivery不能count delivered；unsupported不得返回伪装的SMS message_id。

## 6. 回归与退出

- DDB：后序坏表达式/类型运算/修改主键/过大item使前序Put不可见且stream零；两个barrier竞争条件事务至多一个；普通写与事务竞争、transaction read无撕裂；顺序/并发相同token仅一次增量和stream；内容冲突、TTL边界、容量满首token仍可重放。
- SQS：真实provider创建source+target，首次receive、visibility=0、下次receive触发；目标收到body/source ARN，源不可见；standard/FIFO都测；目标缺失/删除/命令容量满不丢；FIFO碰撞与scope切换。
- Events：PutTargets/List完整SqsParameters roundtrip，真实app bridge到FIFO接收到group=g；无SQS/unsupported明确失败；容量满拒绝、慢目标timeout、panic计failed、quiesce后无接受/变更。
- SNS：真实SQS fanout保留FIFO参数；unsupported protocol/SMS错误；unavailable配置错误；quiesce和失败统计。
- targeted cargo build/test/clippy，nightly rustfmt仅改动文件，不动根Cargo.lock/Toml/Makefile。父负责全workspace门禁和独立review；本worker不委派、不commit。

## 7. 父runtime接线

1. main禁用SQS分支用Unavailable*而非Noop；保留Events/SNS provider Arc于runtime registry。
2. 停止HTTP准入后先Events/SNS quiesce，再SQS handoff quiesce，最后snapshot；保存后SQS shutdown_all。每步消费同一总deadline，失败不得宣称保存一致切面。
3. readiness/capabilities披露SQS依赖与仅SQS目标支持、消息不持久化；stats接diagnostics。此设计不要求app snapshot扩大范围。
