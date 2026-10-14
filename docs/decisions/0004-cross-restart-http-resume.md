<a id="adr-0004-validated-cross-restart-http-resume"></a>
<a id="adr-0004带校验的跨重启-http-续传"></a>

# ADR 0004：带校验的跨重启 HTTP 续传 / Validated Cross-Restart HTTP Resume

- 状态：已接受<br>
  Status: Accepted
- 日期 / Date: 2026-09-28

<a id="context"></a>
<a id="背景"></a>

## 背景 / Context

在本决策之前，Server 会将每个 HTTP 响应暂存到带进程标识的 `.part` 文件。重启后无法安全识别该文件，也无法确认远端响应是否仍是同一份字节，因此恢复会从零开始。大文件下载需要一个受控的重启路径，同时不能把未经校验的部分响应写入目标文件。

Before this decision, the server staged each HTTP response in a process-unique `.part` file. A restart left no safe way to identify that file or verify that a remote response still represented the same bytes, so recovery started from byte zero. Long downloads need a bounded restart path that does not replace the destination with an unvalidated partial response.

<a id="decision"></a>
<a id="决策"></a>

## 决策 / Decision

1. Server 自己拥有的 HTTP Worker 在目标旁派生一个稳定的隐藏部分文件路径，并传给可续传 HTTP Engine。该路径与目标绑定；同一时间只有一个活动任务可以占用该目标。<br>
   The server-owned HTTP worker derives one hidden, stable partial-file path beside the destination and passes it to the resumable HTTP engine. The path is tied to the destination; only one active task may reserve that destination at a time.
2. Engine 在部分文件旁写入原子 JSON sidecar。sidecar 记录来源、目标、响应校验器（`ETag` 或 `Last-Modified`）和预期总长度；部分文件长度就是下一次请求的字节偏移。<br>
   The engine writes an atomic JSON sidecar beside the partial file. The sidecar records the source, destination, response validator (`ETag` or `Last-Modified`), and expected total length. The partial-file length remains the byte offset of the next request.
3. 存在有效部分文件和校验器时，Engine 发送 `Range: bytes=<offset>-` 与 `If-Range`，只接受范围匹配的 `206 Partial Content`。收到 `200 OK`、校验器缺失或变化、`Content-Range` 格式错误或长度不一致时，丢弃部分响应并重新获取完整响应。<br>
   When a valid partial file and validator exist, the engine sends `Range: bytes=<offset>-` with `If-Range` and accepts only a matching `206 Partial Content` range. A `200 OK`, missing or changed validator, malformed `Content-Range`, or inconsistent length discards the partial response and starts a fresh full response.
4. 没有校验器的响应可以在当前进程内完成，但不会在重启后续传。可续传的部分响应在普通 Worker 错误和进程退出后保留。只有完整接收并同步的部分文件才能发布到尚不存在的目标：Engine 优先使用平台提供的原子不覆盖重命名；若不支持，则退回原子硬链接。两种操作都不支持时，提交失败，不会暴露未完成的目标文件。取消、删除任务和成功提交时会尝试清理部分文件与 sidecar，清理错误目前被忽略。<br>
   A response without a validator is allowed to finish in the current process but is not resumed after restart. A resumable partial response is preserved across ordinary worker errors and process exit. Only a complete, synced partial file is published at an absent destination: the engine first uses the platform's atomic no-clobber rename, then an atomic hard-link fallback if that operation is unavailable. If both operations are unsupported, commit fails without exposing an incomplete destination. Cancellation, task removal, and a successful commit attempt to remove the partial file and sidecar; cleanup errors are ignored.
5. SQLite 继续持久化任务状态和进度。sidecar 是 HTTP Engine 拥有的传输会话元数据，因此通用 `TaskRepository` 和同步 `EngineAdapter` 合约不增加 HTTP 专用字段。<br>
   SQLite continues to persist task state and progress. The sidecar is transfer-session metadata owned by the HTTP engine, so the generic `TaskRepository` and synchronous `EngineAdapter` contracts do not gain HTTP-specific fields.

<a id="rationale"></a>
<a id="原因"></a>

## 原因 / Rationale

目标旁的稳定名称可以在不增加 Schema Migration、也不把 Engine 专用字段暴露到每个 Task View 的情况下确定恢复文件。校验器阻止将变化后的远端内容追加到旧文件；完整响应回退则兼容忽略或拒绝 Range 的 Server。sidecar 以原子方式替换，读取时要么是旧的可用元数据，要么不存在，不会是半写状态。

The destination-side name makes recovery deterministic without adding a schema migration or exposing engine-specific fields through every task view. Validators prevent appending bytes from a changed representation, while the full-response fallback handles servers that ignore or reject range requests. Atomic sidecar replacement keeps metadata either old and usable or absent, never half-written.

<a id="consequences"></a>
<a id="影响"></a>

## 影响 / Consequences

- Server 重启后，恢复的 HTTP/HTTPS 任务可以从经过校验的字节偏移继续传输。<br>
  Recovery can resume eligible HTTP/HTTPS work from a verified byte offset after a server restart.
- 未提供稳定 `ETag` 或 `Last-Modified` 的 Server 在中断后仍会从零开始。<br>
  Servers that do not provide a stable `ETag` or `Last-Modified` still restart from byte zero after interruption.
- sidecar 和部分文件必须位于目标旁，并遵守相同的目标占用与清理规则。<br>
  The sidecar and partial file must remain beside the destination and are subject to the same destination ownership and cleanup rules.
- [ADR 0007](0007-http-completion-reconciliation.md) 为 Server 自有路径增加发布前的私有完成凭证、最终进度与 Completed 的原子持久化，以及就绪后的身份／摘要对账，取代原先重启后将已发布完整文件视为目标冲突的限制。未完成响应仍遵循上面的 validator 规则；凭证绑定的暂存文件在完成持久化后才按身份清理。<br>
  [ADR 0007](0007-http-completion-reconciliation.md) extends the Server-owned path with private completion receipts before publication, atomic progress/Completed persistence, and identity/digest reconciliation after readiness. It supersedes the original limitation where a complete published file became a destination conflict after restart. Incomplete responses still follow the validator rules above; receipt-owned staging cleanup waits for durable completion and checks identity.
- 跨主机恢复、Multipart 响应，以及 Magnet/本地文件续传不在本决策范围内。<br>
  Cross-host resume, multipart responses, and resumable magnet or local-file transfers remain outside this decision.

<a id="alternatives-considered"></a>
<a id="被考虑的替代方案"></a>

## 被考虑的替代方案 / Alternatives Considered

- **把校验器持久化到任务 Schema：** 会让通用任务存储和每个客户端视图依赖 HTTP 专用状态；sidecar 可以保持存储合约稳定。<br>
  **Persist validators in the task schema:** would couple generic task storage and every client view to HTTP-specific state; the sidecar keeps the storage contract stable.
- **只按文件长度续传：** 远端内容变化时可能追加错误字节并损坏最终文件。<br>
  **Resume by file length without a validator:** could append bytes from a changed remote representation and corrupt the final file.
- **每个进程使用随机暂存文件：** 重启后无法识别或校验中断的工作。<br>
  **Use a random temporary file per process:** cannot identify or validate interrupted work after restart.
