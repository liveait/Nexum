<a id="adr-0003-cooperative-controls-for-server-http-transfers"></a>
<a id="adr-0003server-http-传输的协作式控制"></a>

# ADR 0003：Server HTTP 传输的协作式控制 / Cooperative Controls for Server HTTP Transfers

- 状态：已接受<br>
  Status: Accepted
- 日期 / Date: 2026-09-26

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

Server 直接运行阻塞式 HTTP Worker，而不是通过同步的 `EngineAdapter::start` 路径运行。此前 Worker 活跃时 Server 会拒绝暂停、恢复和删除。任务协议已经提供这些操作，客户端需要在不替换已有目标文件的情况下安全停止下载。

The server runs blocking HTTP workers directly instead of through the synchronous `EngineAdapter::start` path. Until now it rejected pause, resume, and remove while a worker was active. The task protocol already exposes those operations, and clients need a safe way to stop a download without replacing an existing destination.

跨重启恢复需要稳定的暂存文件名、校验器、Range 请求和持久化传输元数据。同进程的控制边界可以在不修改任务状态模型或存储 Schema 的情况下，先提供可用的暂停与取消语义。

Cross-restart resume would require stable partial-file names, validators, Range requests, and persisted transfer metadata. A smaller same-process control boundary can provide useful pause and cancellation semantics without changing the task state model or storage schema.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

1. 每个 Server HTTP Worker 使用由 Mutex 和 Condvar 支持的共享 `HttpTransferControl`，Worker 在响应块边界检查它。<br>
   Give each server HTTP worker a shared `HttpTransferControl` backed by a mutex and condition variable. The worker checks it at response chunk boundaries.
2. `task.pause` 请求在边界暂停，持久化最新进度，并将任务变为 `Paused`。暂停时保留暂存文件和 HTTP 响应。<br>
   `task.pause` requests a boundary pause, persists the latest progress, and transitions the task to `Paused`. The worker keeps its temporary file and response open while paused.
3. `task.resume` 使用现有 Scheduler 槽位计数并唤醒同一个 Worker。没有可用槽位时返回 `false`，保持现有协议契约。<br>
   `task.resume` uses the existing Scheduler slot accounting and wakes the same worker. It returns `false` when no slot is available, matching the existing protocol contract.
4. `task.remove` 将活跃传输标记为破坏性取消，唤醒 Worker，最多等待 30 秒后通过 `Core` 删除任务。Worker 仍阻塞时请求返回错误，退出后可以重试。取消会丢弃暂存文件，不进入普通失败/重试路径；已有目标文件保持不变。<br>
   `task.remove` marks the active transfer for destructive cancellation, wakes the worker, waits up to 30 seconds for completion, and then removes the task through `Core`. If the worker remains blocked, the request returns an error and can be retried after it exits. Cancellation drops the temporary file and never enters the normal failed/retry path; an existing destination is left untouched.
5. 通用同步 `HttpEngine` Adapter 仍不声明暂停/恢复能力。在 Adapter 支持异步会话 API 前，这些控制属于 Server 自己拥有的 Worker 路径。<br>
   Keep the generic synchronous `HttpEngine` adapter pause/resume capabilities disabled. These controls belong to the server-owned worker path until the adapter supports an asynchronous session API.

<a id="rationale"></a>
<a id="原因"></a>

## 原因 / Rationale

Server 已经同时拥有 Worker、Core 锁、Scheduler 槽位、目标登记和 RPC 请求。协作式边界可以保持这些不变量，又无需增加新的任务状态或执行存储迁移。删除前等待 Worker 完成，可以避免任务删除后仍收到进度、重试或将暂存文件重命名到目标路径。

The server already owns the worker, Core lock, scheduler slot, destination reservation, and RPC request. A cooperative boundary preserves those invariants while avoiding a second task state or a storage migration. Waiting for worker completion before removal prevents a deleted task from receiving progress, retrying, or renaming a completed temporary file afterward.

<a id="consequences"></a>
<a id="影响"></a>

## 影响 / Consequences

- 暂停和恢复只在同一 Server 进程以及 HTTP 响应仍保持有效时生效。远端或网络关闭空闲响应时，恢复仍可能失败。<br>
  Pause and resume work only while the same server process and HTTP response remain alive. A peer or network that closes an idle response can still make resume fail.
- 阻塞中的响应读取无法立即观察 Condvar，因此 `task.pause` 可能要等到 30 分钟 HTTP 请求超时。`task.remove` 最多等待 Worker 30 秒；Worker 仍阻塞时返回错误，退出后可以重试删除。<br>
  A blocking response read cannot observe a condition variable until the 30-minute HTTP request timeout expires, so `task.pause` may be delayed by that timeout. `task.remove` waits up to 30 seconds for the worker and returns an error if it is still blocked; the request can be retried after the worker exits.
- 重启恢复仍会将中断任务归一为 `Queued`；带校验的跨重启 Range 续传由[ADR 0004](0004-cross-restart-http-resume.md)定义，没有可用校验器的响应仍从零开始。<br>
  Restart recovery still normalizes interrupted work to `Queued`; validated cross-restart Range resume is defined by [ADR 0004](0004-cross-restart-http-resume.md), while responses without a usable validator still begin from byte zero.
- 现有 JSON-RPC 返回形状不变：暂停和删除返回 `true`，恢复返回 `true` 或 `false`。<br>
  The existing JSON-RPC shapes remain unchanged: pause and remove return `true`, while resume returns `true` or `false`.

<a id="alternatives-considered"></a>
<a id="被考虑的替代方案"></a>

## 被考虑的替代方案 / Alternatives Considered

- **暂停时取消并重新排队：** 会丢失同进程的部分响应，并让暂停与重试无法区分。<br>
  **Cancel and requeue on every pause:** would lose the same-process partial response and make pause indistinguishable from a retry.
- **现在实现 Range 续传：** 需要持久化暂存文件身份、校验器、响应校验和恢复逻辑，超出本切片范围。<br>
  **Implement Range resume now:** would require persisted partial-file identity, validators, response validation, and recovery changes beyond this slice.
- **通过同步 Adapter 暴露控制：** 需要将 `EngineAdapter::start` 改为异步会话边界，并影响现有 Engine。<br>
  **Expose controls through the synchronous adapter:** would require turning `EngineAdapter::start` into an asynchronous session boundary and affect all existing engines.
