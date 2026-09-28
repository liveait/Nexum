# ADR 0003：Server HTTP 传输的协作式控制

- 状态：已接受
- 日期：2026-09-26

## 背景

Server 直接运行阻塞式 HTTP Worker，而不是通过同步的 `EngineAdapter::start` 路径运行。此前 Worker 活跃时 Server 会拒绝暂停、恢复和删除。任务协议已经提供这些操作，客户端需要在不替换已有目标文件的情况下安全停止下载。

跨重启恢复需要稳定的暂存文件名、校验器、Range 请求和持久化传输元数据。同进程的控制边界可以在不修改任务状态模型或存储 Schema 的情况下，先提供可用的暂停与取消语义。

## 决策

1. 每个 Server HTTP Worker 使用由 Mutex 和 Condvar 支持的共享 `HttpTransferControl`，Worker 在响应块边界检查它。
2. `task.pause` 请求在边界暂停，持久化最新进度，并将任务变为 `Paused`。暂停时保留暂存文件和 HTTP 响应。
3. `task.resume` 使用现有 Scheduler 槽位计数并唤醒同一个 Worker。没有可用槽位时返回 `false`，保持现有协议契约。
4. `task.remove` 将活跃传输标记为破坏性取消，唤醒 Worker，最多等待 30 秒后通过 `Core` 删除任务。Worker 仍阻塞时请求返回错误，退出后可以重试。取消会丢弃暂存文件，不进入普通失败/重试路径；已有目标文件保持不变。
5. 通用同步 `HttpEngine` Adapter 仍不声明暂停/恢复能力。在 Adapter 支持异步会话 API 前，这些控制属于 Server 自己拥有的 Worker 路径。

## 原因

Server 已经同时拥有 Worker、Core 锁、Scheduler 槽位、目标登记和 RPC 请求。协作式边界可以保持这些不变量，又无需增加新的任务状态或执行存储迁移。删除前等待 Worker 完成，可以避免任务删除后仍收到进度、重试或将暂存文件重命名到目标路径。

## 影响

- 暂停和恢复只在同一 Server 进程以及 HTTP 响应仍保持有效时生效。远端或网络关闭空闲响应时，恢复仍可能失败。
- 阻塞中的响应读取无法立即观察 Condvar，因此 `task.pause` 可能要等到 30 分钟 HTTP 请求超时。`task.remove` 最多等待 Worker 30 秒；Worker 仍阻塞时返回错误，退出后可以重试删除。
- 重启恢复仍会将中断任务归一为 `Queued`；带校验的跨重启 Range 续传由[ADR 0004](0004-cross-restart-http-resume.zh-CN.md)定义，没有可用校验器的响应仍从零开始。
- 现有 JSON-RPC 返回形状不变：暂停和删除返回 `true`，恢复返回 `true` 或 `false`。

## 被考虑的替代方案

- **暂停时取消并重新排队：** 会丢失同进程的部分响应，并让暂停与重试无法区分。
- **现在实现 Range 续传：** 需要持久化暂存文件身份、校验器、响应校验和恢复逻辑，超出本切片范围。
- **通过同步 Adapter 暴露控制：** 需要将 `EngineAdapter::start` 改为异步会话边界，并影响现有 Engine。

英文版见 [0003-http-transfer-controls.md](0003-http-transfer-controls.md)。
