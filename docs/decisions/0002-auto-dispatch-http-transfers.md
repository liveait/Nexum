<a id="adr-0002-server-owned-automatic-dispatch-for-http-transfers"></a>
<a id="adr-0002由-server-负责-http-传输自动派发"></a>

# ADR 0002：由 Server 负责 HTTP 传输自动派发 / Server-Owned Automatic Dispatch for HTTP Transfers

- 状态：已接受<br>
  Status: Accepted
- 日期 / Date: 2026-09-26
- 取代：ADR 0001 的第 5 项决策（重试派发保持手动）<br>
  Supersedes: ADR 0001, decision 5 (retry dispatch remains manual)

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

Server 已经负责运行中的 HTTP Worker 以及 Core/Scheduler 状态，而 Scheduler 本身明确不执行网络 I/O。每次 `task.queue`、重试或重启后都要求额外调用 `task.start`，会让持久化队列依赖客户端轮询，也会让可用并发槽位闲置。Server 还需要避免启动不支持的来源，或让两个 HTTP 传输同时写入同一规范化目标路径。

The server already owns the running HTTP worker and the Core/Scheduler state, while the Scheduler deliberately remains free of network I/O. Requiring a separate `task.start` request after every `task.queue`, retry, or restart made the persisted queue dependent on client polling and left available concurrency slots idle. The server also needs to avoid starting unsupported sources or two HTTP transfers that target the same normalized destination.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

1. 将自动派发保留在 Server 层。Scheduler 继续提供队列顺序、任务领取、重试策略和并发计数，但不创建 Worker，也不执行网络 I/O。<br>
   Keep automatic dispatch in the server layer. The Scheduler continues to provide queue ordering, claims, retry policy, and concurrency accounting; it does not spawn workers or perform network I/O.
2. 在 `task.queue` 成功后、启动恢复并开始监听后，以及每个 HTTP Worker 完成或失败后运行 Server 派发器。派发器会持续领取符合条件的 HTTP/HTTPS 排队任务，直到没有 Scheduler 槽位或符合条件的任务。<br>
   Run the server dispatcher after a successful `task.queue`, after startup recovery and listener setup, and after every HTTP worker completion or failure. The dispatcher repeatedly claims eligible queued HTTP/HTTPS tasks until no scheduler slot or eligible task remains.
3. 任务必须解析为 HTTP 或 HTTPS，目标必须位于数据目录外且通过路径安全检查，并且不能与活动目标重叠，才具备派发资格。Magnet 和本地文件任务继续排队，不会阻塞兼容的 HTTP 工作。<br>
   An eligible task must resolve to HTTP or HTTPS, have a permitted destination outside the data directory, and not overlap an active destination. Magnet and local-file tasks remain queued without blocking compatible HTTP work.
4. 保留 `task.start` 作为手动 kick 和兼容接口。它领取一个排队 HTTP/HTTPS 任务，然后调用同一派发器填充剩余槽位。RPC 会在传输结束前返回。<br>
   Keep `task.start` as a manual kick and compatibility method. It claims one queued HTTP/HTTPS task and then invokes the same dispatcher to fill remaining slots. The RPC returns before transfer completion.
5. 释放 Core 锁后再创建 Worker。正常完成或失败时，先释放活动任务和目标路径登记，再重新运行派发器。失败任务是否重新入队仍由已有重试策略决定；只要有可用槽位，派发器就会启动该重试。<br>
   Spawn workers after releasing the Core mutex. On normal completion or failure, release the active task and destination bookkeeping before dispatching again. Existing retry policy determines whether a failed task is requeued; the dispatcher starts that retry when a slot is available.

<a id="rationale"></a>
<a id="原因"></a>

## 原因 / Rationale

Server 是当前唯一同时拥有 RPC 操作、持久化 Core 状态、Scheduler 领取、目标路径安全检查和 HTTP Engine 的组件。将协调逻辑放在这里，可以保持 Scheduler 的库边界，并让入队和恢复不再依赖客户端轮询。让入队、启动、Worker 完成、Worker 失败和手动 kick 共用一个派发器，也能保证并发与目标检查一致。

The server is the only current component that combines RPC operations, persistent Core state, Scheduler claims, destination safety checks, and the HTTP engine. Keeping orchestration there preserves the Scheduler's library boundary and makes queueing and recovery useful without requiring a client-side polling loop. Reusing one dispatcher for queue, startup, worker completion, worker failure, and manual kick keeps concurrency and destination checks consistent.

<a id="consequences"></a>
<a id="影响"></a>

## 影响 / Consequences

- 只要有容量，排队一个符合条件的 HTTP/HTTPS 任务即可启动它。<br>
  Queueing an eligible HTTP/HTTPS task is sufficient to start it when capacity is available.
- Server 重启恢复后，会自动派发符合条件的排队 HTTP/HTTPS 任务。HTTP Worker 按 [ADR 0004](0004-cross-restart-http-resume.md) 对匹配的部分响应使用 `Range`/`If-Range`；不支持、没有校验器或受阻的任务继续排队，未验证的响应从零开始。<br>
  A server restart automatically dispatches eligible queued HTTP/HTTPS tasks after recovery. The HTTP worker resumes a matching validated partial with `Range`/`If-Range`; unsupported, unvalidated, or blocked work remains queued or starts from byte zero as defined by [ADR 0004](0004-cross-restart-http-resume.md).
- HTTP 传输失败时，只要现有重试预算允许，就会自动重试。重试次数和 Scheduler 顺序仍保存在内存中，重启后不会持久化。<br>
  A failed HTTP attempt is automatically retried while the existing retry budget permits. Retry counts and scheduler ordering remain in memory and are not persisted across restart.
- Magnet 和本地文件传输仍没有 Engine。活跃 HTTP 控制由[ADR 0003](0003-http-transfer-controls.md)单独定义；本派发器只负责启动和回收 Worker。<br>
  Magnet and local-file transfers still have no engine. Active HTTP control is defined separately by [ADR 0003](0003-http-transfer-controls.md); this dispatcher only starts and retires workers.
- Worker panic 或 Server Mutex poisoning 可能导致活动登记未释放；这类进程级故障的恢复不在本决策范围内。<br>
  A worker panic or poisoned server mutex can leave active bookkeeping unreleased; recovery for those process-level failures remains outside this decision.

<a id="alternatives-considered"></a>
<a id="被考虑的替代方案"></a>

## 被考虑的替代方案 / Alternatives Considered

- **每次传输都要求新的 `task.start` 请求：** 实现简单，但会让排队工作闲置，并把进度依赖到客户端轮询。<br>
  **Require a new `task.start` request for every attempt:** simple to implement, but leaves queued work idle and couples progress to client polling.
- **在 Scheduler/Core 内创建 Worker：** 会让可复用的 Scheduler 执行网络协调，越过现有 Engine 边界。<br>
  **Put worker spawning inside Scheduler/Core:** would make the reusable scheduler perform network orchestration and cross the existing engine boundary.
- **使用后台定时器轮询排队任务：** 会增加延迟和另一套生命周期循环，却没有利用 Server 已有的事件触发点。<br>
  **Poll for queued work in a background timer:** would add latency and another lifecycle loop without improving the event-driven triggers already available in the server.
