<a id="adr-0003-cooperative-controls-for-server-http-transfers"></a>
<a id="adr-0003server-http-传输的协作式控制"></a>

# ADR 0003：Server HTTP 传输的协作式控制 / Cooperative Controls for Server HTTP Transfers

- 状态：已接受<br>
  Status: Accepted
- 日期 / Date: 2026-09-26

实现参数按 `317dbc035fb60ffe89b0d13e9d2603ef3547ec21` 复核于 2026-11-04。当前会话控制与重启续传的操作说明见[开发指南](../DEVELOPMENT.md#server-and-cli)。

Implementation parameters were checked at `317dbc035fb60ffe89b0d13e9d2603ef3547ec21` on 2026-11-04. See the [Development Guide](../DEVELOPMENT.md#server-and-cli) for current session controls and restart resume.

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
4. `task.remove` 将活跃传输标记为破坏性取消，唤醒 Worker，使用 10 秒的 Worker 完成等待预算，退出后通过 `Core` 删除任务。Worker 仍阻塞时请求返回错误，退出后可以重试。取消路径按续传／完成元数据尝试清理暂存文件，具体边界见 [ADR 0007](0007-http-completion-reconciliation.md)。<br>
   `task.remove` marks the active transfer for destructive cancellation, wakes the worker, and uses a 10-second budget to wait for worker completion before removing the task through `Core`. If the worker remains blocked, the request returns an error and can be retried after it exits. The cancellation path attempts partial-file cleanup under the recovery/completion rules in [ADR 0007](0007-http-completion-reconciliation.md).
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
- 响应体读取在响应块边界及每个 1 秒无数据窗口后检查暂停／取消，连续 8 个读取窗口没有数据时判为停滞失败，收到字节后重置计数。暂停保留当前响应。同进程控制的响应体读取预算、连接／响应头等待、文件 I/O 和最终保存分别受各自边界约束。`task.remove` 的 Worker 完成等待使用 10 秒预算，超时返回可重试错误。<br>
  Response-body reads check pause/cancellation at chunk boundaries and after each one-second no-data window. Eight consecutive empty read windows fail the transfer as stalled, while received bytes reset the counter. Pause retains the current response. Response-body control checks, connection/header waits, file I/O and final persistence have their respective boundaries. The worker-completion wait for `task.remove` uses a 10-second budget and returns a retryable error on timeout.
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
