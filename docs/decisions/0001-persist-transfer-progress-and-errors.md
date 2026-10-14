<a id="adr-0001-persist-transfer-progress-and-errors"></a>
<a id="adr-0001持久化传输进度与错误"></a>

# ADR 0001：持久化传输进度与错误 / Persist Transfer Progress and Errors

- 状态：已接受；第 5 项已由 ADR 0002 取代<br>
  Status: Accepted; decision 5 superseded by ADR 0002
- 日期 / Date: 2026-09-25

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

HTTP Worker 过去只持久化最终字节数，因此长时间传输通过任务 API 看起来没有变化，失败信息也只能在 Server 日志中查看。SQLite Schema Version 1 没有跨重启保存错误的字段。Server 已经负责任务持久化，且已有重试策略，因此本次变更需要沿用现有边界，并兼容已有数据库。

The HTTP worker used to persist only the final byte count. A long-running transfer therefore looked unchanged through the task API, and a failed transfer exposed its message only in the server log. SQLite schema version 1 had no place to retain that error across a restart. The server already owns task persistence and has an existing retry policy, so the change must preserve those boundaries and remain compatible with existing databases.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

1. `HttpEngine` 每写入一个响应块后报告一次进度快照。<br>
   Report a progress snapshot after every response chunk written by `HttpEngine`.
2. Server 自上次写入起累计新增至少 1 MiB 或经过 250 ms 时持久化中间进度；成功任务标记为 `Completed` 前始终持久化最终快照。<br>
   Persist intermediate progress from the server after at least 1 MiB has arrived or 250 ms have elapsed since the previous write. Always persist the final snapshot before marking a successful task `Completed`.
3. 在 `DownloadTask`、`StoredTask` 和 SQLite Schema Version 2 中增加可为空的 `last_error`。Version 1 数据库通过增加空值字段完成迁移。<br>
   Add nullable `last_error` storage to `DownloadTask`, `StoredTask`, and SQLite schema version 2. Migrate version 1 databases by adding the column with a null value.
4. `TaskView` 通过可选的 `error` 字段返回最近一次传输错误。默认重试预算未耗尽时，失败传输会保留错误并重新排队；预算耗尽后任务保持 `Failed`。领取新一轮任务时清除错误并重置进度，成功完成时也清除错误。<br>
   Expose the most recent transfer error as the optional `error` field on `TaskView`. A failed transfer keeps that message while the existing retry budget remains; after the budget is exhausted the task stays `Failed`. Claiming a new attempt clears the error and resets progress. Successful completion also clears it.
5. 暂时保留手动派发重试。排队中的重试仍需再次调用 `task.start`。该临时决策已由 [ADR 0002](0002-auto-dispatch-http-transfers.md) 取代，重试派发现在由 Server 负责的 HTTP 派发器执行。<br>
   Keep retry dispatch manual for now. A queued retry still requires another `task.start` request. This temporary decision is superseded by [ADR 0002](0002-auto-dispatch-http-transfers.md), which moves retry dispatch into the server-owned HTTP dispatcher.

<a id="rationale"></a>
<a id="原因"></a>

## 原因 / Rationale

本方案沿用现有 Core、Scheduler、Repository 和 JSON-RPC 边界。节流可以限制 SQLite 写入次数，同时不影响 Engine 报告进度；最终刷新保证完成任务的字节数准确。可为空的字段让数据库迁移保持追加式，并为旧任务定义空错误值。

This uses the existing Core, scheduler, repository, and JSON-RPC boundaries. Throttling limits SQLite writes without hiding progress from the engine, while the final flush preserves an exact completed byte count. A nullable column makes the database migration additive and gives older tasks a defined empty error value.

<a id="consequences"></a>
<a id="影响"></a>

## 影响 / Consequences

- 客户端可以在传输期间观察进度，并在重启后查看最近一次失败。<br>
  Clients can observe useful progress during a transfer and inspect the last failure after a restart.
- Protocol 增加了可追加的 `error` 字段；客户端应忽略未知响应字段。<br>
  The protocol gains an additive `error` field; clients should ignore unknown response fields.
- 在本决策作出时，传输重启后从零开始，活跃 HTTP 控制尚未实现。之后的同进程控制由 [ADR 0003](0003-http-transfer-controls.md) 定义，当前带校验的跨重启续传由 [ADR 0004](0004-cross-restart-http-resume.md) 定义。<br>
  At the time of this decision, a transfer started from byte zero after restart and active HTTP controls were not implemented. Same-process controls were later defined by [ADR 0003](0003-http-transfer-controls.md), and validated cross-restart resume is now defined by [ADR 0004](0004-cross-restart-http-resume.md).
- 中间进度会增加 SQLite 写入次数，但受字节和时间阈值限制。<br>
  Intermediate progress creates additional SQLite writes, bounded by the byte and time thresholds.

<a id="alternatives-considered"></a>
<a id="被考虑的替代方案"></a>

## 被考虑的替代方案 / Alternatives Considered

- 只持久化最终快照：写入次数较少，但无法提供实时进度或重启可见的传输状态。<br>
  Persist only the final snapshot: fewer writes, but no live progress or restart-visible transfer state.
- 使用独立错误历史表：可以保留更多历史，但当前只需要“最近一次错误”契约，迁移范围也会更大。<br>
  Store errors in a separate history table: richer history, but unnecessary for the current "most recent error" contract and a larger migration.
- 先增加 Server 事件流：有利于推送客户端，但不能替代轮询客户端需要的持久状态或重启恢复。<br>
  Add a server event stream first: useful for push clients, but it would not solve durable state for polling clients or restart recovery.
