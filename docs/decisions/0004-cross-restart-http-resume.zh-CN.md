# ADR 0004：带校验的跨重启 HTTP 续传

- 状态：已接受
- 日期：2026-09-28

## 背景

在本决策之前，Server 会将每个 HTTP 响应暂存到带进程标识的 `.part` 文件。重启后无法安全识别该文件，也无法确认远端响应是否仍是同一份字节，因此恢复会从零开始。大文件下载需要一个受控的重启路径，同时不能把未经校验的部分响应写入目标文件。

## 决策

1. Server 自己拥有的 HTTP Worker 在目标旁派生一个稳定的隐藏部分文件路径，并传给可续传 HTTP Engine。该路径与目标绑定；同一时间只有一个活动任务可以占用该目标。
2. Engine 在部分文件旁写入原子 JSON sidecar。sidecar 记录来源、目标、响应校验器（`ETag` 或 `Last-Modified`）和预期总长度；部分文件长度就是下一次请求的字节偏移。
3. 存在有效部分文件和校验器时，Engine 发送 `Range: bytes=<offset>-` 与 `If-Range`，只接受范围匹配的 `206 Partial Content`。收到 `200 OK`、校验器缺失或变化、`Content-Range` 格式错误或长度不一致时，丢弃部分响应并重新获取完整响应。
4. 没有校验器的响应可以在当前进程内完成，但不会在重启后续传。可续传的部分响应在普通 Worker 错误和进程退出后保留；取消、删除任务和成功替换目标时删除部分文件与 sidecar。
5. SQLite 继续持久化任务状态和进度。sidecar 是 HTTP Engine 拥有的传输会话元数据，因此通用 `TaskRepository` 和同步 `EngineAdapter` 合约不增加 HTTP 专用字段。

## 原因

目标旁的稳定名称可以在不增加 Schema Migration、也不把 Engine 专用字段暴露到每个 Task View 的情况下确定恢复文件。校验器阻止将变化后的远端内容追加到旧文件；完整响应回退则兼容忽略或拒绝 Range 的 Server。sidecar 以原子方式替换，读取时要么是旧的可用元数据，要么不存在，不会是半写状态。

## 影响

- Server 重启后，恢复的 HTTP/HTTPS 任务可以从经过校验的字节偏移继续传输。
- 未提供稳定 `ETag` 或 `Last-Modified` 的 Server 在中断后仍会从零开始。
- sidecar 和部分文件必须位于目标旁，并遵守相同的目标占用与清理规则。
- 跨主机恢复、Multipart 响应，以及 Magnet/本地文件续传不在本决策范围内。

## 被考虑的替代方案

- **把校验器持久化到任务 Schema：** 会让通用任务存储和每个客户端视图依赖 HTTP 专用状态；sidecar 可以保持存储合约稳定。
- **只按文件长度续传：** 远端内容变化时可能追加错误字节并损坏最终文件。
- **每个进程使用随机暂存文件：** 重启后无法识别或校验中断的工作。

英文版见 [0004-cross-restart-http-resume.md](0004-cross-restart-http-resume.md)。
