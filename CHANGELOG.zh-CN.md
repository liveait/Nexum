# 变更日志

此处记录 Nexum 的重要变更。

## 未发布

### 新增

- **Core**：任务生命周期、Scheduler、Resolver 集成与事件收集。SQLite 存储和重启恢复现已接入运行中的 Server；恢复时会将原先下载中、暂停或重试中的任务归一为 `Queued` 并持久化。
- **Engine**：内存与 HTTP 适配器。HTTP 适配器跟随重定向，将响应暂存到 `.part` 文件，在完整下载后重命名到目标路径，在每个响应块写入后报告进度，并为 Server Worker 提供协作式暂停/取消控制。Server 自有的传输路径现在会保留稳定的隐藏部分文件和原子 JSON sidecar，并在重启后仅续传校验器匹配的 `206 Partial Content` 响应。通用同步 Adapter 仍不提供暂停/恢复能力。
- **Protocol**：JSON-RPC 2.0 任务与服务器信息方法、版本标识、凭据字段、事件封装和缓冲类型，以及面向订阅客户端的 `events.event` 通知。`task.get` 和 `task.list` 返回的 `TaskView` 包含持久化进度，以及表示最近一次传输错误的 `error` 字段。Protocol 定义了认证失败错误 `-32001`；Server 会在 RPC 分发和事件订阅前校验配置的 Bearer 或 ApiKey 凭据。
- **Server**：仅监听本机的逐行 TCP JSON-RPC 服务，每个连接使用一个线程。Server 持有数据目录锁，将任务保存在 `data_dir/nexum.sqlite`，并在监听前执行恢复；目录、锁、数据库或恢复失败会阻止启动。排队 HTTP/HTTPS 任务、启动时恢复排队任务，以及活动 HTTP 传输完成或失败，都会触发 Server 自动派发器。派发器遵守 Scheduler 并发上限，只领取符合条件的任务，并跳过不支持的来源和重叠目标。`task.start` 仍是手动 kick，会启动一个 HTTP/HTTPS Worker 后返回，然后使用同一派发器填充其他槽位。响应新增至少 1 MiB 或经过 250 ms 时 Server 持久化中间进度，并在写入 `Completed` 前刷新最终快照。失败时会在 `TaskView` 中保留错误，并在默认三次重试预算未耗尽时重新排队；有可用槽位时派发器会启动重试，预算耗尽后任务保持 `Failed`。新一轮领取会清除旧错误。活跃 HTTP Worker 现在支持响应块边界 `task.pause`、同进程 `task.resume` 和破坏性 `task.remove` 取消；阻塞中的响应读取可能让暂停等到 30 分钟 HTTP 超时，删除最多等待 30 秒后返回错误。重启恢复会对校验器匹配的 HTTP 部分响应使用 `Range`/`If-Range`；校验器缺失或变化、收到 `200`、`Content-Range` 无效、长度不一致，或响应没有校验器时，都会从零开始。客户端可以通过独立 `events.subscribe` TCP 连接接收带序号的 Task/Scheduler 通知；事件流没有回放缓冲，连接或重连后客户端会刷新完整快照。配置的 `max_connections` 会在 TCP 接入时执行，超出上限的连接会立即关闭。启用 `require_auth` 后，Server 会在每个 RPC 和 `events.subscribe` 前要求匹配的 `auth_scheme` 与非空 `auth_token`；缺少或无效凭据会返回 `-32001`（`authentication required`）。`server.auth` 只返回当前 scheme，不会暴露 token；认证配置不完整时会阻止启动。
- **CLI**：任务命令、可复用的 TCP JSON-RPC Client、保存服务器地址与明文凭据的配置、`auth set`/`auth clear` 校验、服务器信息查询、RPC 错误格式化，以及使用系统根证书和主机名校验的 `tls://host:port` 传输。明文凭据只会发给实际回环 peer；TLS 凭据只会在握手成功后发送。
- **Desktop**：通过 TCP 连接 Server 的 Tauri 2 + React 任务界面，提供服务器地址输入、原生目标选择器、独立事件订阅、按连接检测 sequence 缺口、去抖任务刷新，以及重连后完整快照同步的轮询回退。在 macOS 上，通用设置可按 Server 地址将 Bearer/ApiKey 凭据保存到 Keychain 或清除。secret 不会回读到 React；Tauri 为普通 RPC 和 `events.subscribe` 附加凭据。保存、清除或切换当前 Server 会重启事件流并刷新 Task 快照。明文凭据仍仅用于回环地址；已校验的 TLS 地址使用按传输身份隔离的 Keychain 账户；其他平台的 Desktop 仍可无认证连接，但不支持保存或清除凭据。Tauri 的 JavaScript 与 Rust 包已固定为兼容的次版本，macOS bundle ID 也不再以 `.app` 结尾。
- **Browser**：Manifest V3 右键菜单、链接检测和弹窗配置。扩展通过 Server 回环 HTTP `POST /jsonrpc` 桥接发送 `task.create` 和 `task.queue`；通过校验的扩展 Origin 可收到 JSON 请求的 CORS 头，网页 Origin 会被拒绝，事件订阅仍只走 TCP，凭据设置尚未提供。
- **Security**：凭据、TLS 和限流配置类型。Server 现在会按配置执行 Bearer/ApiKey 鉴权，以及由 TCP 与 HTTP `/jsonrpc` 共用一个令牌桶的可选进程级 RPC 限流。限流默认关闭，启用时须提供完整、有效的速率和突发容量参数。已解析且通过认证的请求（包括 `events.subscribe`）消耗令牌；认证失败、事件 heartbeat 和 HTTP `OPTIONS` 预检不消耗。带 `id` 的超额调用返回 JSON-RPC `-32002`（HTTP 状态码 `200`）；无 `id` 的通知会被丢弃，不返回 JSON-RPC 响应。限流配置无效或不完整，以及显式传入的 `--config` 文件无法读取，会阻止启动。Server 现在支持成对配置 PEM 证书和私钥，让 TCP JSON-RPC、事件订阅和 HTTP `/jsonrpc` 使用 TLS；CLI 和 Desktop 会校验 `tls://` Server 名称和系统根证书，Browser 客户端信任接入仍待完成。
- **Plugin**：Manifest、Permission、Capability、生命周期状态及 Provider Trait 基础；Core 可记录插件 Manifest，但尚未加载 Provider 提供的 Engine 或 Resolver。
- **Media**：媒体与工作流模型，以及模拟的内存探测和任务执行；尚未接入实际媒体处理。

### 文档

- 同步 README、架构、开发指南和合并后的开发计划，使其与当前仓库实现状态一致，并记录认证配置合约与 CLI 凭据流程。
- 保持英文与简体中文项目文档同步。
- 记录 Dependabot 的 `glib 0.18.5` 告警，以及 GTK 3/Tauri 约束如何阻止不安全地强制替换为 `glib 0.20`；Tauri 2.12/wry 0.57 在 Linux 上仍使用受影响的 GTK 3 版本线。

### 修复

- **CI**：移除重复的 Workflow，并修复 Workspace 兼容性问题，使主 CI 完成格式化、检查、测试与 Clippy 校验。
- **Branding**：刷新 Desktop、Browser Extension 与 macOS 图标资源，并修复无效的 PNG / ICNS 数据。

## [0.1.0] - TBD

- 初始项目结构与公开开发基础。

中文文档请参见项目中的简体中文文档。
