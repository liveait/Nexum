<a id="architecture"></a>
<a id="架构设计"></a>

# 架构设计 / Architecture

导航 / Contents: [运行边界 / Runtime Boundary](#runtime-boundary) · [Crate 职责 / Crates and Responsibilities](#crates-and-responsibilities) · [任务路径 / Task Flow](#task-flow) · [Engine 与来源 / Engines and Sources](#engines-and-sources) · [持久化与恢复 / Persistence and Recovery](#persistence-and-recovery) · [Protocol 与客户端 / Protocol and Clients](#protocol-and-clients) · [扩展与 Media 边界 / Extension and Media Boundaries](#extension-and-media-boundaries)

本文描述仓库中已经存在的行为。crate 中定义了类型或 trait，并不代表 Server 或客户端已经接入该能力。

This document describes the behavior present in the repository. A type or trait in a crate does not imply that the server or clients use it yet.

当前验收边界（2026-10-14）：基线 `3768c3a` 的新 HTTP 会话／分段路径存在范围验证、文件保护、最终进度和完成恢复回归。下文仍保留先前路径的保护契约作为修复约束，不代表新路径已完整继承或通过验收；以[开发指南的状态说明](DEVELOPMENT.md#development-workflow-entry)和最终修复 SHA 的验证为准。

Current acceptance boundary (2026-10-14): the new HTTP session/segment paths at `3768c3a` regress range checks, file protection, final progress, and completion recovery. The protection contracts from the prior paths remain below as repair requirements; they do not establish that the new paths preserve them or have passed acceptance. Use the [Development Guide status](DEVELOPMENT.md#development-workflow-entry) and verification at the final fixed SHA.

<a id="runtime-boundary"></a>
<a id="运行边界"></a>

## 运行边界 / Runtime Boundary

```text
CLI ───────────────┐
Desktop (Tauri) ───┼── line-delimited JSON-RPC 2.0 / TCP
                   ▼
              Local server
              /                    \
      RpcDispatcher       HTTP dispatch coordinator
              │                    │              \
              └────────────► Core/Scheduler   HTTP worker ── HttpEngine ── destination
                                      ▲              │
                                      └── progress/final result ─┘
                                      │
                              event pump / EventHub
                                      │
                         events.subscribe TCP stream
                                      ▼
                              Desktop event client
                 ┌─────┼───────────┐
  TaskService   Scheduler   TaskRepository
       │            │              │
  Task events   Queue/events   SQLite (server)
                    │           InMemory (injectable)
                    ▼
              EngineRegistry (Core library API)
             /              \
       InMemoryEngine     HttpEngine

Browser extension ── HTTP(S) POST /jsonrpc + CORS (same listener)
```

图示术语 / Diagram terms: Local server = 本地 Server；HTTP dispatch coordinator = HTTP 派发协调器；HTTP worker = HTTP Worker；destination = 目标文件；progress/final result = 进度/最终结果；event pump = 事件泵；TCP stream = TCP 流；Desktop event client = Desktop 事件客户端；InMemory (injectable) = 可注入内存实现；Core library API = Core 库 API；Browser extension = Browser 扩展。

Server 默认监听 `127.0.0.1:39100`。当前实现始终绑定回环地址，`--port` 只改变端口。CLI 可以连接不带 scheme 的 TCP 地址，也可以用 `tls://host:port` 选择 TLS；Desktop UI 可以填写 Server 地址。同一个监听器同时接受按行分隔的 TCP JSON-RPC 和单请求 HTTP(S) `POST /jsonrpc`。HTTP 请求必须使用 `Content-Type: application/json` 并提供有上限的 `Content-Length`；分块请求和事件订阅仍不支持 HTTP(S) 传输。只有通过校验的 Chrome、Firefox 或 Safari 扩展 Origin 会收到 CORS 响应；网页 Origin 会在 RPC 分发前被拒绝。

The server listens on `127.0.0.1:39100` by default. Its address is always bound to loopback in the current implementation; `--port` changes only the port. The CLI can connect to a configured bare TCP address or select TLS with `tls://host:port`, and the desktop UI accepts a server address. The same listener accepts line-delimited TCP JSON-RPC and one-request HTTP(S) `POST /jsonrpc` calls. HTTP requests require `Content-Type: application/json` and a bounded `Content-Length`; chunked requests and event subscriptions remain unsupported over HTTP(S). CORS responses are emitted only for validated Chrome, Firefox, or Safari extension origins; web-page origins are rejected before RPC dispatch.

Desktop 管理者已调用 Server 的专用 `--managed` 入口：后端校验包内可执行文件 SHA-256、私有管道就绪消息中的 PID／构建／协议身份，再执行带鉴权的 `server.version`，之后才开放具体任务操作。每次启动的随机令牌只保存在 Rust 内存与私有管道中。管理者保有子进程句柄和管道，限制崩溃重启次数，退出 App 或切换外部模式时只停止自己启动的子进程。关闭窗口只隐藏界面，Dock 可重新打开。托管数据位于 `app_data_dir()/server/`，与 CLI／开发数据独立。Server 在随机 IPv4 回环端口监听，stdout 只报告一次就绪，Shutdown／EOF 后退出并保留部分文件。契约和剩余验收见 [ADR 0006](decisions/0006-managed-macos-server.md)。

The Desktop supervisor calls the dedicated `--managed` entry point. It verifies the bundled executable SHA-256, readiness PID/build/protocol identity and authenticated `server.version` before routing task commands. Only Rust holds the per-launch token. The supervisor owns the child handle and pipes, limits crash restarts, and stops only its child on Quit or external-mode selection. Closing the window hides it; Dock reopen restores it. Managed storage uses `app_data_dir()/server/`, independently of CLI/development data. The Server validates private stdin frames, listens on an ephemeral loopback port, reports one secret-free readiness frame, and exits on Shutdown/EOF while retaining partial files. See [ADR 0006](decisions/0006-managed-macos-server.md).

<a id="crates-and-responsibilities"></a>
<a id="crate-职责"></a>

## Crate 职责 / Crates and Responsibilities

| Crate | 已实现职责<br>Implemented role |
| --- | --- |
| `domain` | Task ID、来源、目标路径和进度值类型。<br>Task ID, source, destination, and progress value types. |
| `task` | 内存中的任务服务、经过校验的状态转换和 Task Events。<br>In-memory task service, validated state transitions, and task events. |
| `scheduler` | 显式队列操作、优先级、并发上限、重试策略、带宽策略计算和 Scheduler Events。不执行网络 I/O；Server 派发协调器使用它的领取与并发限制启动兼容的 HTTP 工作。<br>Explicit queue operations, priority, concurrency limit, retry policy, bandwidth-policy calculation, and scheduler events. It does not perform network I/O; the server dispatch coordinator uses its claims and limits to start compatible HTTP work. |
| `storage` | `TaskRepository` 的内存与 SQLite 实现；SQLite 包含 Schema Version 和 Migration。<br>`TaskRepository` with in-memory and SQLite implementations. SQLite has a schema version and migration. |
| `resolver` | 通过 Registry 对 HTTP/HTTPS、Magnet 和本地来源分类、校验。<br>HTTP/HTTPS, magnet, and local-source classification and validation through a registry. |
| `engine` | Adapter/Registry API、模拟的 InMemory Engine 和阻塞式 HTTP GET Engine。<br>Adapter/registry APIs, simulated in-memory engine, and blocking HTTP GET engine. |
| `core` | 协调 Task、Scheduler、Resolver、Engine、Repository 与 Plugin Manager 状态。<br>Coordinates task, scheduler, resolver, engine, repository, and plugin manager state. |
| `security` | Credential、TLS 配置、限流配置和 Credential Store 类型。<br>Credential, TLS configuration, rate-limit, and credential-store types. |
| `protocol` | JSON-RPC 请求/响应分发、Task/Server 方法、错误对象与事件信封。<br>JSON-RPC request/response dispatch, task/server methods, error objects, and event envelopes. |
| `plugin` | Manifest/Permission/Capability 类型、Provider Trait 和 Plugin Manager 状态机。<br>Manifest/permission/capability types, provider traits, and a plugin-manager state machine. |
| `media` | 媒体与工作流数据类型、依赖排序、模拟的内存 Job。<br>Media and workflow data types, dependency ordering, and simulated in-memory jobs. |

<a id="task-flow"></a>
<a id="任务路径"></a>

## 任务路径 / Task Flow

`Core::create_task` 经 `ResolverRegistry` 校验来源，创建 `DownloadTask` 并写入注入的 Repository。`queue_task` 将任务加入 Scheduler。Server 派发协调器会在 `task.queue`、启动恢复以及 HTTP Worker 完成或失败后运行。它反复选取具有允许且不重叠目标的最高优先级 HTTP/HTTPS 排队任务，占用 Scheduler 并发槽，重置进度、清除旧错误，并在 Core 锁之外启动 Worker，直到没有可用槽位或符合条件的任务。`task.start` 仍是手动 kick 和兼容接口：领取一个排队 HTTP/HTTPS 任务，在传输结束前返回任务 ID，然后调用同一填充循环。Magnet 和本地文件任务因没有兼容的传输 Engine 而继续排队，不会阻塞排在其后的 HTTP/HTTPS 任务。`Core::start_next` 对库调用者仍默认使用内存 Engine；`start_next_with_engine("http")` 也是库 API。

`Core::create_task` validates the source through `ResolverRegistry`, creates a `DownloadTask`, and writes it to the injected repository. `queue_task` adds it to the scheduler. The server dispatch coordinator runs after `task.queue`, after startup recovery, and after an HTTP worker completes or fails. It repeatedly selects the highest-priority queued HTTP/HTTPS task with a permitted, non-overlapping destination, claims a scheduler slot, resets its progress, clears the previous transfer error, and spawns the worker outside the Core mutex until no slot or eligible task remains. `task.start` remains a manual kick and compatibility method: it claims one queued HTTP/HTTPS task, returns its ID before transfer completion, and then invokes the same fill loop. Magnet and local-file tasks remain queued because no transfer engine supports them; they do not block compatible HTTP/HTTPS tasks behind them. `Core::start_next` remains an in-memory-engine default for library callers; `start_next_with_engine("http")` is also a library API.

任务状态机允许：

The task state machine permits:

```text
Created     → Queued
Queued      → Downloading
Downloading → Paused | Completed | Failed
Paused      → Queued | Downloading
Completed   → Queued
Failed      → Retrying → Queued
```

Scheduler 在领取任务或恢复任务时检查并发数。HTTP Worker 每写入一个响应块就报告进度；Server 在新增至少 1 MiB 或经过 250 ms 时持久化中间快照，并通过 `Core::complete_task` 的一次 repository update 同时保存最终进度和 `Completed`；保存失败不会发布候选任务／Scheduler 状态或事件。传输失败后，Scheduler 在默认三次重试预算内重新入队，Server 保存最近一次错误；有可用槽位时派发协调器会自动启动重试。`task.get` 和 `task.list` 通过 `error` 字段返回最近一次传输错误，新一轮领取任务时会重置任务快照、清除旧错误，并由 Server 恢复可用的部分进度；预算耗尽后任务保持 `Failed`。带宽策略目前只计算限速值，HTTP 传输并未应用。Task 和 Scheduler Events 在 Core 中收集，由 Server Event Pump 取出并发布给已订阅的客户端。

The scheduler enforces its concurrency count when a task is claimed or resumed. `HttpEngine` reports a progress snapshot after each response chunk. The server persists an intermediate snapshot when at least 1 MiB has arrived since the previous write or 250 ms have elapsed, and persists final progress together with `Completed` in one `Core::complete_task` repository update. A rejected update publishes no candidate task/scheduler state or events. On transfer failure, the server stores the latest error, while the scheduler requeues the task while the default three-retry budget remains; the dispatch coordinator starts the retry when a slot is available. A new claim clears the previous error and resets the task snapshot before the server restores any validated partial progress. After the retry budget is exhausted, the task stays `Failed`. Bandwidth policy currently calculates limits; the HTTP transfer does not apply them. Task and scheduler events are collected in Core, drained by a server event pump, and published to subscribed clients.

<a id="engines-and-sources"></a>
<a id="engine-与来源"></a>

## Engine 与来源 / Engines and Sources

Resolver 接受 HTTP/HTTPS URL、包含 `xt=urn:btih:` 参数的 Magnet URI，以及存在的本地路径；它不会校验 Magnet Hash 本身。Server 仅将 HTTP/HTTPS 路由到传输 Engine，并拒绝数据目录内的目标、符号链接目标，以及指向同一规范路径的并发活动传输。`HttpEngine` 执行阻塞式 GET，最多跟随五次重定向，连接超时为 10 秒，请求超时为 30 分钟，并在每个响应块写入后报告进度。Server 自有的 HTTP Worker 会在每个目标旁保留一个隐藏的稳定部分文件，并用原子 JSON sidecar 保存来源、目标、ETag 或 Last-Modified 校验器及预期长度。重启后只会对匹配的元数据发送 `Range` 与 `If-Range`，并只接受匹配的 `206 Partial Content` 范围。收到 `200`、校验器变化或缺失、`Content-Range` 格式错误或长度不一致时，会删除部分响应并重新完整下载。没有校验器且尚未完整接收的响应在重启后从零开始；已经在 EOF 证明完整的响应可通过私有完成凭证对账。带校验的部分数据会在普通传输错误和进程退出期间保留；成功提交、取消和删除任务时会尝试清理它与 sidecar。完整响应接收并同步后，部分文件会原子地以目标文件名出现；若目标是无关的已有文件，任务失败且不覆盖原文件。Server 的直接 HTTP Worker 路径支持在响应块边界协作式暂停、同一进程内恢复，以及破坏性删除取消。暂停时保留部分文件和响应，删除时取消 Worker 并删除数据库任务。普通未完成暂存数据可在取消时清理；完成凭证绑定的暂存文件在数据库删除后按身份检查，已有目标文件保持不变。阻塞中的响应读取可能让 `task.pause` 等到 30 分钟请求超时。`task.remove` 最多等待 Worker 30 秒，Worker 未退出时返回错误；Worker 可能继续阻塞到 HTTP 超时，退出后可以重试删除。通用 `HttpEngine` Adapter 仍是同步接口，不声明暂停/恢复能力。InMemory Engine 只模拟生命周期，不传输字节。当前没有 Magnet 或本地文件传输 Engine。

The resolver accepts HTTP/HTTPS URLs, magnet URIs containing an `xt=urn:btih:` parameter, and existing local paths; it does not validate the magnet hash itself. The server routes only HTTP/HTTPS to a transfer engine. It rejects destinations inside the server data directory, symbolic-link destinations, and concurrent active transfers targeting the same canonical path. `HttpEngine` performs a blocking GET, follows up to five redirects, has a 10-second connection timeout and a 30-minute request timeout, and reports progress after each written response chunk. The server-owned HTTP worker keeps one hidden stable partial file beside each destination and an atomic JSON sidecar containing the source, destination, ETag or Last-Modified validator, and expected length. After restart it sends `Range` and `If-Range` only for matching metadata, and accepts only a matching `206 Partial Content` range. A `200` response, changed or missing validator, malformed `Content-Range`, or inconsistent length removes the partial response and starts a full download. An interrupted incomplete response without a validator restarts from byte zero. A response proved complete at EOF can instead be reconciled from its private completion receipt. Validated partial data survives ordinary transfer errors and process exit; successful commit, cancellation, and task removal attempt to clean it up. The completed partial file is synced and exposed at the destination atomically without replacing an existing path; an unrelated existing destination fails the task. The server's direct HTTP worker path supports cooperative pause at a response chunk boundary, same-process resume, and destructive remove cancellation. Pause retains the partial file and response while the worker waits; remove cancels the worker and removes the database task. Ordinary incomplete staging may be cleaned during cancellation; receipt-bound staging is checked by identity after database removal, and existing destinations remain in place. A blocking response read can delay `task.pause` until the 30-minute request timeout. `task.remove` waits up to 30 seconds for the worker and returns an error if it has not exited; the worker can remain blocked until the HTTP timeout, after which removal can be retried. The generic `HttpEngine` adapter remains synchronous and advertises no pause/resume capability. The in-memory engine simulates lifecycle operations and does not transfer bytes. There is no magnet or local-file transfer engine.

提交时优先使用平台的原子不覆盖重命名（macOS 为 `renamex_np`，Linux 为 `renameat2`，Windows 为不启用覆盖的 `MoveFileExW`）。若该操作不可用，Engine 尝试原子硬链接；若文件系统两种操作都不支持，提交会失败，不会暴露未完成的目标文件。

Commit uses the platform's atomic no-clobber rename (`renamex_np` on macOS, `renameat2` on Linux, and `MoveFileExW` without replacement on Windows). If that operation is unavailable, the engine tries an atomic hard link. If the filesystem supports neither operation, commit fails without exposing an incomplete destination.

Server 自有 engine 在发布前写入 `data_dir/http-completions/` 下有大小上限的私有凭证，绑定任务 ID、来源、路径、实际字节数、可选总长度、文件身份及 SHA-256。Worker 在访问网络前对账：最终文件匹配时完成任务，目标不存在且完整暂存文件匹配时重新发布。校验拒绝无关身份、内容变化、符号链接和特殊文件。全量摘要计算放在就绪之后，配合暂停／取消。一次最终进度／Completed 持久化成功后，先通过有界身份检查清理暂存文件、校验小型 sidecar，再删除凭证；保存或清理失败保留凭证。删除任务先删除数据库任务再清理；清理错误可能留下孤儿凭证，复用 ID 前须先清除。最终目标保持不变。详见 [ADR 0007](decisions/0007-http-completion-reconciliation.md)。

Before publication, the Server-owned engine writes a bounded private receipt under `data_dir/http-completions/`, binding task ID, source, paths, actual bytes, optional total, file identity and SHA-256. The worker checks that receipt before network access: matching final bytes finish the task, or matching complete staging bytes can be published if the final path is absent. Receipt validation rejects unrelated identities, changed content, symlinks and special files. Full-file hashing runs after readiness and cooperates with pause/cancel. After one successful progress/Completed persistence, bounded identity-based staging cleanup and small sidecar verification precede receipt removal. A persistence or cleanup failure preserves the receipt. Task removal deletes the database task before cleanup; a cleanup error can leave orphan evidence, and reuse of its ID requires clearing that evidence first. Final destinations are preserved. See [ADR 0007](decisions/0007-http-completion-reconciliation.md).

<a id="persistence-and-recovery"></a>
<a id="持久化与恢复"></a>

## 持久化与恢复 / Persistence and Recovery

`Core<R>` 接受 `TaskRepository`。Server 在进程运行期间锁住 `data_dir/nexum.lock`，在 `data_dir/nexum.sqlite`（默认 `./data/nexum.sqlite`）打开数据库，并在接受连接前调用 `Core::recover_with_pending_completion`。第二个使用同一数据目录的 Server 无法启动。SQLite Repository 持久化任务元数据、进度和最近一次传输错误 `last_error`（Schema Version 2）。恢复时，已创建和已完成任务保持原状态；失败任务通常保持失败，只有正确绑定的有界完成凭证才会将其选入 worker 对账；已排队任务继续排队，原先下载中、暂停或重试中的任务在内存与 SQLite 中重置为 `Queued`。Scheduler 队列按普通优先级重建；原优先级、顺序和重试次数不持久化。恢复完成并开始监听后，派发协调器会自动启动符合条件的排队 HTTP/HTTPS 任务；不支持的来源或受阻的目标会继续排队。重启后的 HTTP 传输会在派发前恢复经过校验的部分字节；只有 HTTP 校验器和 `206` 范围匹配时才从该字节继续，否则会删除部分响应并从零开始。创建目录、获取锁、打开数据库或恢复失败会使 Server 启动失败。

`Core<R>` accepts a `TaskRepository`. The server locks `data_dir/nexum.lock` for its lifetime, opens `data_dir/nexum.sqlite` (default `./data/nexum.sqlite`), and calls `Core::recover_with_pending_completion` before accepting connections. A second server using the same data directory cannot start. The SQLite repository persists task metadata, progress snapshots, and the most recent transfer error. Recovery keeps created and completed tasks in their stored states; failed tasks remain failed unless a bounded, correctly bound completion receipt selects them for worker reconciliation; queued tasks remain queued, while previously downloading, paused, or retrying tasks are reset to `Queued` in memory and SQLite. The scheduler queue is rebuilt at normal priority; prior priority, order, and retry counts are not persisted. After recovery and listener startup, the dispatch coordinator automatically starts eligible queued HTTP/HTTPS tasks; unsupported sources or blocked destinations remain queued. A restarted HTTP transfer restores its validated partial byte count before dispatch. It resumes from that byte only when the HTTP validator and `206` range match; otherwise the partial response is discarded and the transfer starts from byte zero. Directory creation, lock acquisition, database opening, and recovery failures stop server startup.

Desktop 设置选择托管（默认）或外部模式。所有前端任务和事件命令在 Rust 中解析路由，不接受调用方指定端点；Keychain 操作仅限外部模式。后端校验 HTTP(S) URL 和文件名、生成任务 ID，并绕开已有任务、完整文件、部分文件及 sidecar 分配带序号的文件名。分配过程与设置切换串行，不创建最终文件，Engine 的不覆盖提交保护继续有效。远程外部目标只检查任务占用，不检查这台 Mac 的文件系统。

Desktop settings select Managed (default) or External. Every frontend task and event command resolves its route in Rust rather than accepting a caller-supplied endpoint. Keychain operations are restricted to External mode. The backend validates the HTTP(S) URL and filename, generates task IDs, and allocates a numbered filename around existing tasks, final files, partial files, and sidecars. Allocation is serialized with settings switches, does not create the final file, and retains the Engine’s no-clobber commit protection. Remote External targets are checked against task reservations without inspecting this Mac’s filesystem.

<a id="protocol-and-clients"></a>
<a id="protocol-与客户端"></a>

## Protocol 与客户端 / Protocol and Clients

Server 每次从 TCP 连接读取一行 JSON-RPC 请求，对带 `id` 的请求写回一行响应。它会在 Dispatcher 之前处理 `task.start` 和活跃 HTTP 控制；Dispatcher 支持 `task.get`、`task.list`、`task.create`、`task.queue`、`task.start`、`task.pause`、`task.resume`、`task.remove`、`server.version` 和 `server.auth`，没有通用的 Task Update 方法。对 Server 的活跃 HTTP Worker，`task.pause` 会等待响应块边界确认并持久化为 `Paused`，`task.resume` 在有 Scheduler 槽位时唤醒同一 Worker，`task.remove` 会取消 Worker，最多等待 30 秒后删除任务。超时删除会返回错误，Worker 退出后可以重试。`task.queue` 成功后会在 Core 操作结束时派发符合条件的 HTTP/HTTPS 工作；`task.start` 仍可手动 kick 一个排队任务。`task.get` 和 `task.list` 返回带当前持久化进度以及最近一次传输错误 `error` 字段的 `TaskView`；新一轮领取任务时会清除该字段。Protocol 包含 V1 版本字段和版本查询方法，但没有协商功能集合；除 JSON-RPC `2.0` 信封校验外，也没有版本强制校验。客户端可以建立独立 TCP 连接发送 `events.subscribe`，收到确认后持续接收 `events.event` JSON-RPC 通知。非 heartbeat 通知携带增量 Task 或 Scheduler 数据，并使用单调递增的 Server 序号；Server 每 15 秒发送一次 heartbeat，复用当前序号，不代表任务更新。每个订阅者使用有界队列，队列满时会断开订阅。当前没有回放缓冲：客户端应将断线或前进方向的 sequence 缺口视为状态过期，重连并获取完整快照；同一连接内重复或过期的 sequence 可以忽略。

The server reads one JSON-RPC request per TCP line and writes a response line for requests with IDs. It handles `task.start` and active HTTP control before the dispatcher; the dispatcher supports `task.get`, `task.list`, `task.create`, `task.queue`, `task.start`, `task.pause`, `task.resume`, `task.remove`, `server.version`, and `server.auth`. A successful `task.queue` dispatches eligible HTTP/HTTPS work after the Core operation; `task.start` remains available to manually kick one queued task. For an active server HTTP worker, `task.pause` waits for a chunk-boundary acknowledgement and persists `Paused`, `task.resume` wakes the same worker when a scheduler slot is available, and `task.remove` cancels the worker, waits up to 30 seconds for it to exit, and deletes the task after that wait succeeds. A timed-out removal returns an error and can be retried after the worker exits. `task.get` and `task.list` return `TaskView` values with the current persisted progress and an `error` field for the most recent transfer error; the field is cleared when a new attempt is claimed. There is no general task-update method. The protocol has a V1 version field and a version-inspection method, but no negotiated feature set or version enforcement beyond JSON-RPC `2.0` envelope validation. A client can open a dedicated TCP connection, send `events.subscribe`, receive an acknowledgement, and then receive `events.event` JSON-RPC notifications. Non-heartbeat notifications carry incremental task or scheduler data and have monotonically increasing server sequences; a heartbeat is sent every 15 seconds, reuses the current sequence, and does not represent a task update. The per-subscriber queue is bounded; a full queue disconnects that subscriber. There is no replay buffer: clients treat a disconnect or forward sequence gap as stale state, reconnect, and fetch a full snapshot; duplicate or stale sequence values on one connection are ignored.

CLI 通过 TCP 协议管理任务、查询 Server。不带 scheme 的 `host:port` 保持明文兼容路径；`tls://host:port` 使用 rustls 客户端，加载平台根证书（包括 `rustls-native-certs` 支持的 `SSL_CERT_FILE`/`SSL_CERT_DIR` 来源），并在发送 RPC 或凭据前校验 Server 名称。Tauri 2 + React Desktop 通过 Tauri 命令调用 TCP JSON-RPC，使用 Tauri 后台线程建立独立事件订阅，按连接跟踪 event sequence，忽略重复或过期通知，在发现前进方向缺口时重连，并在收到 Task 或 Scheduler 通知后去抖刷新任务快照；重连或 sequence 缺口会安排一次完整快照，事件流断开时按已配置的策略回退到空闲每五秒、下载中每秒的轮询，手动策略会禁用该回退。窗口隐藏时暂停轮询；Server 地址、刷新策略和语言选择通过 Tauri 持久化。托管或回环外部模式下，每次新建下载默认使用已保存的本机目录或系统 Downloads 文件夹，可用原生文件夹选择器改选，并预览由可编辑的推测文件名组成的目标路径。连接远程 Server 时，用户需输入 Server 主机上的绝对文件夹路径；App 不调用这台 Mac 的选择器或本机文件存在性检查。通用设置显示当前已保存 Server 的凭据 scheme，并允许保存或清除 secret；凭据变化会重启事件订阅并刷新 Task 快照。Desktop 键盘导航与快捷键、弹窗焦点管理、可访问性标签和减少动效行为已接入。CSS 通过 `prefers-color-scheme` 跟随系统明暗外观；React 使用类型化的中英文文案目录展示 Desktop 自有的可见及无障碍文字。已保存的语言选择可以跟随 WebView 报告的首选系统语言（中文归为简体中文，其他语言使用英文），也可以显式覆盖；旧设置默认跟随系统。该展示选择不改变协议值或 Server 请求，Server/操作系统的原始诊断信息保持原文。手动主题和强调色选项仍待实现。Manifest V3 Browser 扩展通过 HTTP 桥接发送 `task.create` 和 `task.queue`，读取 Popup 写入的 `server` 键，将不带 scheme 的地址映射到 HTTP、将显式 `tls://host:port` 映射到浏览器 HTTPS，接收链接标记消息，并在发送前解析相对链接。浏览器负责正常的证书和主机名校验，TLS 请求失败时扩展不会改用 HTTP 重试。扩展仍没有设备选择或凭据设置，因此无法使用要求认证的 Server。

The CLI uses the TCP protocol for task control and server inspection. A bare `host:port` keeps the plaintext compatibility path; `tls://host:port` creates a rustls client using the platform root store (including the `SSL_CERT_FILE`/`SSL_CERT_DIR` sources supported by `rustls-native-certs`) and validates the server name before any RPC or credential is sent. The Tauri 2 + React desktop app uses Tauri commands as a TCP JSON-RPC client, with a Motrix-style sidebar, Downloads filters, task Inspector, Add Download sheet, Settings, Notifications, and planned-capability pages. It opens a dedicated event subscription through a Tauri background thread, tracks event sequences per connection, ignores duplicate or stale notifications, reconnects on a forward gap, refreshes the task snapshot after debounced task or scheduler notifications, and uses the configured five-second idle or one-second active polling fallback when the stream is disconnected. A reconnect or sequence gap schedules a full snapshot; manual refresh policy disables the polling fallback. It pauses polling when the window is hidden and persists the Server address, refresh policy, and language choice through Tauri. For Managed or loopback External mode, each new download defaults to the saved local folder or this Mac's system Downloads folder; the user can use the native folder picker and preview the destination assembled from the editable suggested file name. For a remote Server, the user enters an absolute folder path on the Server host; the App does not use this Mac's folder picker or local file-existence check. General settings show the active saved Server's configured credential scheme and allow saving or clearing its secret; those changes restart the event subscription and refresh the task snapshot. Desktop keyboard navigation, shortcuts, modal focus management, accessible labels, and reduced-motion behavior are implemented. CSS follows the system light/dark preference through `prefers-color-scheme`; React uses a typed English/Simplified Chinese message catalog for Desktop-owned visible and accessibility-facing text. The saved language choice can follow the first preferred language reported by the WebView (Chinese resolves to Simplified Chinese, other languages to English) or override it; older settings default to following the system. This presentation choice does not alter protocol values or Server requests, and raw Server/OS diagnostics remain verbatim. Manual theme and accent-color options remain planned. The Manifest V3 browser extension uses the HTTP bridge for `task.create` and `task.queue`, reads the popup's `server` storage key, maps bare addresses to HTTP and explicit `tls://host:port` addresses to browser HTTPS, handles link-badge messages, and resolves relative links before sending. The browser performs normal certificate and hostname validation, and the extension does not retry a failed TLS request over HTTP. It has no device selection or credential settings, so an authenticated server rejects its requests.

Protocol 请求可以携带一个 `Credential`（`Bearer` 或 `ApiKey`）。启用 `require_auth` 后，Server 会在进入任何 Dispatcher 方法或处理 `events.subscribe` 前校验请求凭据。配置的 scheme 和 secret 必须精确匹配；缺少、无效或不匹配的凭据会返回 JSON-RPC 错误 `-32001`，消息为 `authentication required`。`require_auth=true` 要求配置受支持的 `auth_scheme` 和非空 `auth_token`；配置不完整或不受支持时 Server 会拒绝启动。`server.auth` 只返回当前 scheme（`none`、`Bearer` 或 `ApiKey`），不会暴露 secret。CLI 将 `default_auth_scheme` 和 `default_auth_token` 以明文保存在配置中；CLI 只会在已校验的 `tls://` RPC 连接上，或确认实际 peer 为回环地址的明文连接上附加凭据。在 macOS 上，Tauri 后端按当前已保存 Server 地址将 Desktop 的 scheme 与 secret 保存到 Keychain，并为普通 RPC 和 `events.subscribe` 附加凭据；查询 Keychain 状态的命令只返回已配置的 scheme，React 不会回读已保存的 secret。Desktop 对 `tls://` 连接使用系统根证书并在发送凭据前校验 Server 名称，Keychain 账户按传输身份隔离；明文凭据传输仍只允许回环地址。其他平台的 Desktop 可以无认证连接，但不支持保存和清除凭据。Server 不会记录凭据 secret。`max_connections` 会限制活动 TCP 连接处理器，超过上限的连接在处理请求前关闭。Server 配置成对的 `tls_cert_path` 和 `tls_key_path` 后会启用 TLS；监听器在接受连接前加载 PEM 材料，TCP JSON-RPC、`events.subscribe` 和 HTTP `/jsonrpc` 共用同一 rustls 流。分阶段 TLS 传输契约及实施边界记录在 [ADR 0005](decisions/0005-tls-transport.md)。

Protocol requests can carry one `Credential` value (`Bearer` or `ApiKey`). When `require_auth` is enabled, the server validates the request credential before any dispatcher method or `events.subscribe` handling. The configured scheme and secret must match exactly; missing, invalid, or mismatched credentials return JSON-RPC error `-32001` with message `authentication required`. `require_auth=true` requires a supported `auth_scheme` and non-empty `auth_token`; an incomplete or unsupported pair stops startup. `server.auth` reports the active scheme (`none`, `Bearer`, or `ApiKey`) without exposing the secret. The CLI stores `default_auth_scheme` and `default_auth_token` as plain text in its config. It attaches them to verified `tls://` RPC requests, while plaintext credentials are allowed only after the connected peer is confirmed to be loopback. On macOS, the Tauri backend stores the Desktop credential's scheme and secret under the active saved Server address in Keychain and attaches it to ordinary RPC and `events.subscribe`; the Keychain status command returns only the configured scheme, so React never reads back the saved secret. Desktop uses the system root store for `tls://` connections, validates the server name before sending credentials, and scopes Keychain accounts by transport identity; plaintext credential transport remains loopback-only. On other platforms, Desktop can connect without authentication but credential save and clear are unsupported. The server never logs credential secrets. Its `max_connections` setting limits active TCP connection handlers and closes excess connections before request processing. Server TLS can be enabled with paired `tls_cert_path` and `tls_key_path` settings; the listener loads the PEM material before accepting connections and carries TCP JSON-RPC, `events.subscribe`, and HTTP `/jsonrpc` over the same rustls stream. The staged TLS transport contract and rollout boundaries are recorded in [ADR 0005](decisions/0005-tls-transport.md).

可选的 RPC 限流使用一个由逐行 TCP 与 HTTP `POST /jsonrpc` 共用的进程级令牌桶。限流默认关闭；启用时必须同时设置 `--rate-limit-rps` 和 `--rate-limit-burst`，或在配置文件中同时设置 `rate_limit_rps` 和 `rate_limit_burst`，且数值必须为有效正值。配置无效或不完整，以及显式传入的 `--config` 文件无法读取，都会阻止启动。每个已解析并通过认证的请求都会消耗令牌，包括 `events.subscribe`；认证失败、Server 生成的事件 heartbeat 和 HTTP `OPTIONS` 预检不消耗令牌。令牌用尽时，带 `id` 的调用返回 JSON-RPC 错误 `-32002`；HTTP `/jsonrpc` 仍使用状态码 `200`，错误位于响应体中。没有 `id` 的通知会被丢弃，不返回 JSON-RPC 响应。

Optional RPC rate limiting uses one process-wide token bucket shared by line-delimited TCP and HTTP `POST /jsonrpc`. It is disabled by default; the server requires both `--rate-limit-rps` and `--rate-limit-burst`, or both `rate_limit_rps` and `rate_limit_burst` in its config file, with valid positive values. Invalid or incomplete settings stop startup, as does an unreadable file explicitly supplied with `--config`. After parsing and authentication, every request, including `events.subscribe`, consumes a token. Authentication failures, server-generated event heartbeats, and HTTP `OPTIONS` preflight do not consume tokens. When the bucket has no token, calls with an `id` receive JSON-RPC error `-32002`; HTTP `/jsonrpc` keeps status `200` and carries that error in the response body. Notifications without an `id` are dropped without a JSON-RPC response.

<a id="extension-and-media-boundaries"></a>
<a id="扩展与-media-边界"></a>

## 扩展与 Media 边界 / Extension and Media Boundaries

`PluginManager` 可以注册 Manifest 并记录加载、启动、停止状态，crate 定义了 `EngineProvider` 与 `ResolverProvider` Trait。Core 未调用插件实现或注册插件提供的 Adapter；`init_plugins` 只推进 Manager 状态。Permission 声明目前只是元数据，没有沙箱约束。动态加载和可执行插件集成尚未实现。

`PluginManager` registers manifests and tracks load/start/stop states, and the crate defines `EngineProvider` and `ResolverProvider` traits. Core does not call plugin implementations or register their adapters; `init_plugins` only advances manager state. Permission declarations are metadata, not an enforced sandbox. Dynamic loading and executable plugin integration are not present.

Media crate 定义了 Probe、Manifest、Track、Segment、Mux、Pipeline、Job、Workflow 和 MCP Request 类型。Workflow 依赖排序与内存 Job 管理已经实现。`MediaProcessor::probe` 返回默认元数据，Job 执行会构造模拟的完成结果；真实文件探测、Manifest 解析、分段处理、Mux、外部自动化端点和 MCP 集成均未接入。

The media crate defines probe, manifest, track, segment, mux, pipeline, job, workflow, and MCP request types. Workflow dependency ordering and in-memory job bookkeeping are implemented. `MediaProcessor::probe` returns default metadata, while job execution fabricates a completed result; no file probing, manifest parsing, segment processing, muxing, external automation endpoint, or MCP integration is wired up.
