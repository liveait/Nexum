# 变更日志

此处记录 Nexum 的重要变更。

## 未发布

### 新增

- **Core**：任务生命周期、Scheduler、Resolver 集成与事件收集。SQLite 存储和重启恢复现已接入运行中的 Server；恢复时会将原先下载中、暂停或重试中的任务归一为 `Queued` 并持久化。
- **Engine**：内存与 HTTP 适配器。HTTP 适配器跟随重定向，将响应暂存到 `.part` 文件，在完整下载后重命名到目标路径，在每个响应块写入后报告进度，并为 Server Worker 提供协作式暂停/取消控制。Server 自有的传输路径现在会保留稳定的隐藏部分文件和原子 JSON sidecar，并在重启后仅续传校验器匹配的 `206 Partial Content` 响应。通用同步 Adapter 仍不提供暂停/恢复能力。
- **Protocol**：JSON-RPC 2.0 任务与服务器信息方法、版本标识、凭据字段，以及事件封装和缓冲类型。`task.get` 和 `task.list` 返回的 `TaskView` 包含持久化进度，以及表示最近一次传输错误的 `error` 字段。Server 尚不推送事件，也不校验凭据。
- **Server**：仅监听本机的逐行 TCP JSON-RPC 服务，每个连接使用一个线程。Server 持有数据目录锁，将任务保存在 `data_dir/nexum.sqlite`，并在监听前执行恢复；目录、锁、数据库或恢复失败会阻止启动。排队 HTTP/HTTPS 任务、启动时恢复排队任务，以及活动 HTTP 传输完成或失败，都会触发 Server 自动派发器。派发器遵守 Scheduler 并发上限，只领取符合条件的任务，并跳过不支持的来源和重叠目标。`task.start` 仍是手动 kick，会启动一个 HTTP/HTTPS Worker 后返回，然后使用同一派发器填充其他槽位。响应新增至少 1 MiB 或经过 250 ms 时 Server 持久化中间进度，并在写入 `Completed` 前刷新最终快照。失败时会在 `TaskView` 中保留错误，并在默认三次重试预算未耗尽时重新排队；有可用槽位时派发器会启动重试，预算耗尽后任务保持 `Failed`。新一轮领取会清除旧错误。活跃 HTTP Worker 现在支持响应块边界 `task.pause`、同进程 `task.resume` 和破坏性 `task.remove` 取消；阻塞中的响应读取可能让暂停等到 30 分钟 HTTP 超时，删除最多等待 30 秒后返回错误。重启恢复会对校验器匹配的 HTTP 部分响应使用 `Range`/`If-Range`；校验器缺失或变化、收到 `200`、`Content-Range` 无效、长度不一致，或响应没有校验器时，都会从零开始。连接数上限与强制认证尚未执行。
- **CLI**：任务命令、可复用的 TCP JSON-RPC Client、保存服务器地址与凭据的配置、服务器信息查询和 RPC 错误格式化。
- **Desktop**：通过 TCP 连接 Server 的 Tauri 2 + React 任务界面，提供服务器地址输入和手动刷新。
- **Browser**：Manifest V3 右键菜单、链接检测和弹窗配置；其 HTTP 发送请求目前无法与仅支持 TCP 的 Server 配合。
- **Security**：凭据、TLS 和限流配置类型；Server 尚未实现鉴权、TLS 或限流。
- **Plugin**：Manifest、Permission、Capability、生命周期状态及 Provider Trait 基础；Core 可记录插件 Manifest，但尚未加载 Provider 提供的 Engine 或 Resolver。
- **Media**：媒体与工作流模型，以及模拟的内存探测和任务执行；尚未接入实际媒体处理。

### 文档

- 同步 README、架构、开发指南和合并后的开发计划，使其与当前仓库实现状态一致。
- 保持英文与简体中文项目文档同步。

### 修复

- **CI**：移除重复的 Workflow，并修复 Workspace 兼容性问题，使主 CI 完成格式化、检查、测试与 Clippy 校验。
- **Branding**：刷新 Desktop、Browser Extension 与 macOS 图标资源，并修复无效的 PNG / ICNS 数据。

## [0.1.0] - TBD

- 初始项目结构与公开开发基础。

中文文档请参见项目中的简体中文文档。
