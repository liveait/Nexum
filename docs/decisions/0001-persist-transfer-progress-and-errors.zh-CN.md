# ADR 0001：持久化传输进度与错误

- 状态：已接受；第 5 项已由 ADR 0002 取代
- 日期：2026-09-25

## 背景

HTTP Worker 过去只持久化最终字节数，因此长时间传输通过任务 API 看起来没有变化，失败信息也只能在 Server 日志中查看。SQLite Schema Version 1 没有跨重启保存错误的字段。Server 已经负责任务持久化，且已有重试策略，因此本次变更需要沿用现有边界，并兼容已有数据库。

## 决策

1. `HttpEngine` 每写入一个响应块后报告一次进度快照。
2. Server 自上次写入起累计新增至少 1 MiB 或经过 250 ms 时持久化中间进度；成功任务标记为 `Completed` 前始终持久化最终快照。
3. 在 `DownloadTask`、`StoredTask` 和 SQLite Schema Version 2 中增加可为空的 `last_error`。Version 1 数据库通过增加空值字段完成迁移。
4. `TaskView` 通过可选的 `error` 字段返回最近一次传输错误。默认重试预算未耗尽时，失败传输会保留错误并重新排队；预算耗尽后任务保持 `Failed`。领取新一轮任务时清除错误并重置进度，成功完成时也清除错误。
5. 暂时保留手动派发重试。排队中的重试仍需再次调用 `task.start`。该临时决策已由 [ADR 0002](0002-auto-dispatch-http-transfers.zh-CN.md) 取代，重试派发现在由 Server 负责的 HTTP 派发器执行。

## 原因

本方案沿用现有 Core、Scheduler、Repository 和 JSON-RPC 边界。节流可以限制 SQLite 写入次数，同时不影响 Engine 报告进度；最终刷新保证完成任务的字节数准确。可为空的字段让数据库迁移保持追加式，并为旧任务定义空错误值。

## 影响

- 客户端可以在传输期间观察进度，并在重启后查看最近一次失败。
- Protocol 增加了可追加的 `error` 字段；客户端应忽略未知响应字段。
- 在本决策作出时，传输重启后从零开始，活跃 HTTP 控制尚未实现。之后的同进程控制由 [ADR 0003](0003-http-transfer-controls.zh-CN.md) 定义，当前带校验的跨重启续传由 [ADR 0004](0004-cross-restart-http-resume.zh-CN.md) 定义。
- 中间进度会增加 SQLite 写入次数，但受字节和时间阈值限制。

## 被考虑的替代方案

- 只持久化最终快照：写入次数较少，但无法提供实时进度或重启可见的传输状态。
- 使用独立错误历史表：可以保留更多历史，但当前只需要“最近一次错误”契约，迁移范围也会更大。
- 先增加 Server 事件流：有利于推送客户端，但不能替代轮询客户端需要的持久状态或重启恢复。

英文版见 [0001-persist-transfer-progress-and-errors.md](0001-persist-transfer-progress-and-errors.md)。
