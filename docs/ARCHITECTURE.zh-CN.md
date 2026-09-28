# 架构设计

本文描述仓库中已经存在的行为。crate 中定义了类型或 trait，并不代表 Server 或客户端已经接入该能力。

## 运行边界

```text
CLI ───────────────┐
Desktop (Tauri) ───┼── 基于 TCP 的按行 JSON-RPC 2.0
                   ▼
               本地 Server
              /                    \
      RpcDispatcher       HTTP 派发协调器
              │                    │              \
              └────────────► Core/Scheduler   HTTP Worker ── HttpEngine ── 目标文件
                                      ▲              │
                                      └── 进度/最终结果 ──┘
                 ┌─────┼───────────┐
  TaskService   Scheduler   TaskRepository
       │            │              │
  Task Events   Queue/Events  SQLite (Server)
                    │           InMemory (可注入)
                    ▼
              EngineRegistry（Core 库 API）
             /              \
       InMemoryEngine     HttpEngine

Browser 扩展 ── HTTP POST /jsonrpc（Server 尚无对应端点）
```

Server 默认监听 `127.0.0.1:39100`。当前实现始终绑定回环地址，`--port` 只改变端口。CLI 可以连接配置的 TCP 地址，Desktop UI 可以填写 Server 地址。Browser 扩展发送的 HTTP 请求与当前仅支持 TCP 行协议的 Server 不兼容。

## Crate 职责

| Crate | 已实现职责 |
| --- | --- |
| `domain` | Task ID、来源、目标路径和进度值类型。 |
| `task` | 内存中的任务服务、经过校验的状态转换和 Task Events。 |
| `scheduler` | 显式队列操作、优先级、并发上限、重试策略、带宽策略计算和 Scheduler Events。不执行网络 I/O；Server 派发协调器使用它的领取与并发限制启动兼容的 HTTP 工作。 |
| `storage` | `TaskRepository` 的内存与 SQLite 实现；SQLite 包含 Schema Version 和 Migration。 |
| `resolver` | 通过 Registry 对 HTTP/HTTPS、Magnet 和本地来源分类、校验。 |
| `engine` | Adapter/Registry API、模拟的 InMemory Engine 和阻塞式 HTTP GET Engine。 |
| `core` | 协调 Task、Scheduler、Resolver、Engine、Repository 与 Plugin Manager 状态。 |
| `security` | Credential、TLS 配置、限流配置和 Credential Store 类型。 |
| `protocol` | JSON-RPC 请求/响应分发、Task/Server 方法、错误对象与事件信封。 |
| `plugin` | Manifest/Permission/Capability 类型、Provider Trait 和 Plugin Manager 状态机。 |
| `media` | 媒体与工作流数据类型、依赖排序、模拟的内存 Job。 |

## 任务路径

`Core::create_task` 经 `ResolverRegistry` 校验来源，创建 `DownloadTask` 并写入注入的 Repository。`queue_task` 将任务加入 Scheduler。Server 派发协调器会在 `task.queue`、启动恢复以及 HTTP Worker 完成或失败后运行。它反复选取具有允许且不重叠目标的最高优先级 HTTP/HTTPS 排队任务，占用 Scheduler 并发槽，重置进度、清除旧错误，并在 Core 锁之外启动 Worker，直到没有可用槽位或符合条件的任务。`task.start` 仍是手动 kick 和兼容接口：领取一个排队 HTTP/HTTPS 任务，在传输结束前返回任务 ID，然后调用同一填充循环。Magnet 和本地文件任务因没有兼容的传输 Engine 而继续排队，不会阻塞排在其后的 HTTP/HTTPS 任务。`Core::start_next` 对库调用者仍默认使用内存 Engine；`start_next_with_engine("http")` 也是库 API。

任务状态机允许：

```text
Created     → Queued
Queued      → Downloading
Downloading → Paused | Completed | Failed
Paused      → Queued | Downloading
Completed   → Queued
Failed      → Retrying → Queued
```

Scheduler 在领取任务或恢复任务时检查并发数。HTTP Worker 每写入一个响应块就报告进度；Server 在新增至少 1 MiB 或经过 250 ms 时持久化中间快照，并在标记 `Completed` 前刷新最终字节数。传输失败后，Scheduler 在默认三次重试预算内重新入队，Server 保存最近一次错误；有可用槽位时派发协调器会自动启动重试。`task.get` 和 `task.list` 通过 `error` 字段返回最近一次传输错误，新一轮领取任务时会重置任务快照、清除旧错误，并由 Server 恢复可用的部分进度；预算耗尽后任务保持 `Failed`。带宽策略目前只计算限速值，HTTP 传输并未应用。Task 和 Scheduler Events 在 Core 中收集、可取出，但 Server 没有向客户端发布。

## Engine 与来源

Resolver 接受 HTTP/HTTPS URL、包含 `xt=urn:btih:` 参数的 Magnet URI，以及存在的本地路径；它不会校验 Magnet Hash 本身。Server 仅将 HTTP/HTTPS 路由到传输 Engine，并拒绝数据目录内的目标、符号链接目标，以及指向同一规范路径的并发活动传输。`HttpEngine` 执行阻塞式 GET，最多跟随五次重定向，连接超时为 10 秒，请求超时为 30 分钟，并在每个响应块写入后报告进度。Server 自有的 HTTP Worker 会在每个目标旁保留一个隐藏的稳定部分文件，并用原子 JSON sidecar 保存来源、目标、ETag 或 Last-Modified 校验器及预期长度。重启后只会对匹配的元数据发送 `Range` 与 `If-Range`，并只接受匹配的 `206 Partial Content` 范围。收到 `200`、校验器变化或缺失、`Content-Range` 格式错误或长度不一致时，会删除部分响应并重新完整下载。没有校验器的响应可以在当前进程完成，但不能在重启后续传。带校验的部分数据会在普通传输错误和进程退出期间保留；成功提交、取消和删除任务时会清理它与 sidecar。完整响应接收后才会同步并替换目标文件。Server 的直接 HTTP Worker 路径支持在响应块边界协作式暂停、同一进程内恢复，以及破坏性删除取消。暂停时保留部分文件和响应，删除时取消 Worker、丢弃部分文件与 sidecar、删除任务并保留已有目标文件。阻塞中的响应读取可能让 `task.pause` 等到 30 分钟请求超时。`task.remove` 最多等待 Worker 30 秒，Worker 未退出时返回错误；Worker 可能继续阻塞到 HTTP 超时，退出后可以重试删除。通用 `HttpEngine` Adapter 仍是同步接口，不声明暂停/恢复能力。InMemory Engine 只模拟生命周期，不传输字节。当前没有 Magnet 或本地文件传输 Engine。

## 持久化与恢复

`Core<R>` 接受 `TaskRepository`。Server 在进程运行期间锁住 `data_dir/nexum.lock`，在 `data_dir/nexum.sqlite`（默认 `./data/nexum.sqlite`）打开数据库，并在接受连接前调用 `Core::recover`。第二个使用同一数据目录的 Server 无法启动。SQLite Repository 持久化任务元数据、进度和最近一次传输错误 `last_error`（Schema Version 2）。恢复时，已创建、已完成和失败的任务保持原状态；已排队任务继续排队，原先下载中、暂停或重试中的任务在内存与 SQLite 中重置为 `Queued`。Scheduler 队列按普通优先级重建；原优先级、顺序和重试次数不持久化。恢复完成并开始监听后，派发协调器会自动启动符合条件的排队 HTTP/HTTPS 任务；不支持的来源或受阻的目标会继续排队。重启后的 HTTP 传输会在派发前恢复经过校验的部分字节；只有 HTTP 校验器和 `206` 范围匹配时才从该字节继续，否则会删除部分响应并从零开始。创建目录、获取锁、打开数据库或恢复失败会使 Server 启动失败。

## Protocol 与客户端

Server 每次从 TCP 连接读取一行 JSON-RPC 请求，对带 `id` 的请求写回一行响应。它会在 Dispatcher 之前处理 `task.start` 和活跃 HTTP 控制；Dispatcher 支持 `task.get`、`task.list`、`task.create`、`task.queue`、`task.start`、`task.pause`、`task.resume`、`task.remove`、`server.version` 和 `server.auth`，没有通用的 Task Update 方法。对 Server 的活跃 HTTP Worker，`task.pause` 会等待响应块边界确认并持久化为 `Paused`，`task.resume` 在有 Scheduler 槽位时唤醒同一 Worker，`task.remove` 会取消 Worker，最多等待 30 秒后删除任务。超时删除会返回错误，Worker 退出后可以重试。`task.queue` 成功后会在 Core 操作结束时派发符合条件的 HTTP/HTTPS 工作；`task.start` 仍可手动 kick 一个排队任务。`task.get` 和 `task.list` 返回带当前持久化进度以及最近一次传输错误 `error` 字段的 `TaskView`；新一轮领取任务时会清除该字段。Protocol 包含 V1 版本字段和版本查询方法，但没有协商功能集合；除 JSON-RPC `2.0` 信封校验外，也没有版本强制校验。crate 中已有事件信封转换和缓存，Server 尚无订阅或推送通道。

CLI 通过 TCP 协议管理任务、查询 Server。Tauri 2 + React Desktop 通过 Tauri 命令调用 TCP JSON-RPC，包含 Motrix 风格侧边栏、Downloads 过滤、任务 Inspector、Add Download Sheet、Settings、Notifications 和计划能力页面。空闲时每五秒轮询，下载中每秒轮询；断开或窗口隐藏时停止轮询；Server 地址和刷新策略通过 Tauri 持久化，并通过 dialog 插件的原生保存面板选择目标路径。当前仍没有事件推送、快捷键层和本地化 Message Catalog。Manifest V3 Browser 扩展有右键菜单和链接标记 UI，但发送流程向 `/jsonrpc` 发 HTTP 请求，目前没有兼容端点。Popup 写入 Storage 的 `server` 键，后台脚本却读取 `address` 字段，因此保存的地址不会生效。扩展也没有设备选择。

Protocol 请求可以携带 Credential，`server.auth` 目前只返回 `none`。Server 不校验 Credential；`require_auth` 与 `max_connections` 配置虽可解析，但未执行限制。TLS 和限流类型也未接入 Server。

## 扩展与 Media 边界

`PluginManager` 可以注册 Manifest 并记录加载、启动、停止状态，crate 定义了 `EngineProvider` 与 `ResolverProvider` Trait。Core 未调用插件实现或注册插件提供的 Adapter；`init_plugins` 只推进 Manager 状态。Permission 声明目前只是元数据，没有沙箱约束。动态加载和可执行插件集成尚未实现。

Media crate 定义了 Probe、Manifest、Track、Segment、Mux、Pipeline、Job、Workflow 和 MCP Request 类型。Workflow 依赖排序与内存 Job 管理已经实现。`MediaProcessor::probe` 返回默认元数据，Job 执行会构造模拟的完成结果；真实文件探测、Manifest 解析、分段处理、Mux、外部自动化端点和 MCP 集成均未接入。
