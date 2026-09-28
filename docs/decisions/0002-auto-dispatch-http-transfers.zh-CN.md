# ADR 0002：由 Server 负责 HTTP 传输自动派发

- 状态：已接受
- 日期：2026-09-26
- 取代：ADR 0001 的第 5 项决策（重试派发保持手动）

## 背景

Server 已经负责运行中的 HTTP Worker 以及 Core/Scheduler 状态，而 Scheduler 本身明确不执行网络 I/O。每次 `task.queue`、重试或重启后都要求额外调用 `task.start`，会让持久化队列依赖客户端轮询，也会让可用并发槽位闲置。Server 还需要避免启动不支持的来源，或让两个 HTTP 传输同时写入同一规范化目标路径。

## 决策

1. 将自动派发保留在 Server 层。Scheduler 继续提供队列顺序、任务领取、重试策略和并发计数，但不创建 Worker，也不执行网络 I/O。
2. 在 `task.queue` 成功后、启动恢复并开始监听后，以及每个 HTTP Worker 完成或失败后运行 Server 派发器。派发器会持续领取符合条件的 HTTP/HTTPS 排队任务，直到没有 Scheduler 槽位或符合条件的任务。
3. 任务必须解析为 HTTP 或 HTTPS，目标必须位于数据目录外且通过路径安全检查，并且不能与活动目标重叠，才具备派发资格。Magnet 和本地文件任务继续排队，不会阻塞兼容的 HTTP 工作。
4. 保留 `task.start` 作为手动 kick 和兼容接口。它领取一个排队 HTTP/HTTPS 任务，然后调用同一派发器填充剩余槽位。RPC 会在传输结束前返回。
5. 释放 Core 锁后再创建 Worker。正常完成或失败时，先释放活动任务和目标路径登记，再重新运行派发器。失败任务是否重新入队仍由已有重试策略决定；只要有可用槽位，派发器就会启动该重试。

## 原因

Server 是当前唯一同时拥有 RPC 操作、持久化 Core 状态、Scheduler 领取、目标路径安全检查和 HTTP Engine 的组件。将协调逻辑放在这里，可以保持 Scheduler 的库边界，并让入队和恢复不再依赖客户端轮询。让入队、启动、Worker 完成、Worker 失败和手动 kick 共用一个派发器，也能保证并发与目标检查一致。

## 影响

- 只要有容量，排队一个符合条件的 HTTP/HTTPS 任务即可启动它。
- Server 重启恢复后，会自动派发符合条件的排队 HTTP/HTTPS 任务。HTTP Worker 按 [ADR 0004](0004-cross-restart-http-resume.zh-CN.md) 对匹配的部分响应使用 `Range`/`If-Range`；不支持、没有校验器或受阻的任务继续排队，未验证的响应从零开始。
- HTTP 传输失败时，只要现有重试预算允许，就会自动重试。重试次数和 Scheduler 顺序仍保存在内存中，重启后不会持久化。
- Magnet 和本地文件传输仍没有 Engine。活跃 HTTP 控制由[ADR 0003](0003-http-transfer-controls.zh-CN.md)单独定义；本派发器只负责启动和回收 Worker。
- Worker panic 或 Server Mutex poisoning 可能导致活动登记未释放；这类进程级故障的恢复不在本决策范围内。

## 被考虑的替代方案

- **每次传输都要求新的 `task.start` 请求：** 实现简单，但会让排队工作闲置，并把进度依赖到客户端轮询。
- **在 Scheduler/Core 内创建 Worker：** 会让可复用的 Scheduler 执行网络协调，越过现有 Engine 边界。
- **使用后台定时器轮询排队任务：** 会增加延迟和另一套生命周期循环，却没有利用 Server 已有的事件触发点。

英文版见 [0002-auto-dispatch-http-transfers.md](0002-auto-dispatch-http-transfers.md)。
